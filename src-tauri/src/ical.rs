//! Lectura/escritura mínima de iCalendar (RFC 5545) para CalDAV y feeds ICS:
//! desplegado de líneas, parámetros, fechas con TZID, duración, expansión de
//! recurrencias (RRULE/RDATE/EXDATE/RECURRENCE-ID) y serialización.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::models::{date_to_utc, NewEvent, RemoteEvent};

#[derive(Clone, Debug)]
pub struct Prop {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub value: String,
}

impl Prop {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Component {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Component>,
}

impl Component {
    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }
    pub fn props_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Prop> + 'a {
        self.props.iter().filter(move |p| p.name == name)
    }
}

/// Une las líneas plegadas (las que empiezan por espacio o tabulador).
pub fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

pub fn parse_line(line: &str) -> Option<Prop> {
    // Separar nombre;params de valor respetando comillas en los parámetros.
    let mut in_quotes = false;
    let mut colon = None;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = Vec::new();
    let mut cur = String::new();
    in_quotes = false;
    for ch in head.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    let name = parts.first()?.trim().to_ascii_uppercase();
    let params = parts[1..]
        .iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_uppercase(), v.trim().to_string()))
        })
        .collect();
    Some(Prop {
        name,
        params,
        value: value.to_string(),
    })
}

/// Analiza el texto y devuelve los componentes de primer nivel (normalmente VCALENDAR).
pub fn parse(text: &str) -> Vec<Component> {
    let mut stack: Vec<Component> = Vec::new();
    let mut roots = Vec::new();
    for line in unfold(text) {
        let Some(p) = parse_line(&line) else { continue };
        if p.name == "BEGIN" {
            stack.push(Component {
                name: p.value.trim().to_ascii_uppercase(),
                ..Default::default()
            });
        } else if p.name == "END" {
            if let Some(done) = stack.pop() {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => roots.push(done),
                }
            }
        } else if let Some(cur) = stack.last_mut() {
            cur.props.push(p);
        }
    }
    roots
}

pub fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

// ---------- Fechas ----------

#[derive(Clone, Debug, PartialEq)]
pub enum ITime {
    Date(NaiveDate),
    Utc(DateTime<Utc>),
    Zoned(NaiveDateTime, Tz),
    Floating(NaiveDateTime),
}

/// Zona horaria local del sistema (en Flatpak se hereda de /etc/localtime).
pub fn local_tz() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|n| n.parse::<Tz>().ok())
        .unwrap_or(Tz::UTC)
}

fn resolve_local(naive: &NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    match tz.from_local_datetime(naive) {
        chrono::LocalResult::Single(d) => d.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(a, _) => a.with_timezone(&Utc),
        // Hueco por cambio de horario: se avanza una hora.
        chrono::LocalResult::None => tz
            .from_local_datetime(&(*naive + Duration::hours(1)))
            .earliest()
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|| naive.and_utc()),
    }
}

impl ITime {
    pub fn to_utc(&self) -> DateTime<Utc> {
        match self {
            ITime::Date(d) => date_to_utc(*d),
            ITime::Utc(d) => *d,
            ITime::Zoned(n, tz) => resolve_local(n, *tz),
            ITime::Floating(n) => resolve_local(n, local_tz()),
        }
    }
    pub fn is_date(&self) -> bool {
        matches!(self, ITime::Date(_))
    }
}

/// Zonas de Windows más frecuentes (Outlook/Exchange) → IANA.
fn windows_tz(name: &str) -> Option<&'static str> {
    Some(match name {
        "UTC" | "Coordinated Universal Time" | "GMT Standard Time" => "Europe/London",
        "Romance Standard Time" => "Europe/Paris",
        "W. Europe Standard Time" => "Europe/Berlin",
        "Central Europe Standard Time" => "Europe/Budapest",
        "Central European Standard Time" => "Europe/Warsaw",
        "E. Europe Standard Time" => "Europe/Chisinau",
        "FLE Standard Time" => "Europe/Kiev",
        "GTB Standard Time" => "Europe/Bucharest",
        "Russian Standard Time" => "Europe/Moscow",
        "Eastern Standard Time" => "America/New_York",
        "Central Standard Time" => "America/Chicago",
        "Mountain Standard Time" => "America/Denver",
        "US Mountain Standard Time" => "America/Phoenix",
        "Pacific Standard Time" => "America/Los_Angeles",
        "Alaskan Standard Time" => "America/Anchorage",
        "Hawaiian Standard Time" => "Pacific/Honolulu",
        "Atlantic Standard Time" => "America/Halifax",
        "Central Standard Time (Mexico)" => "America/Mexico_City",
        "Mountain Standard Time (Mexico)" => "America/Mazatlan",
        "Central America Standard Time" => "America/Guatemala",
        "SA Pacific Standard Time" => "America/Bogota",
        "Venezuela Standard Time" => "America/Caracas",
        "SA Western Standard Time" => "America/La_Paz",
        "Pacific SA Standard Time" => "America/Santiago",
        "Argentina Standard Time" => "America/Argentina/Buenos_Aires",
        "E. South America Standard Time" => "America/Sao_Paulo",
        "SA Eastern Standard Time" => "America/Cayenne",
        "Montevideo Standard Time" => "America/Montevideo",
        "Paraguay Standard Time" => "America/Asuncion",
        "Peru Standard Time" => "America/Lima",
        "India Standard Time" => "Asia/Kolkata",
        "China Standard Time" => "Asia/Shanghai",
        "Tokyo Standard Time" => "Asia/Tokyo",
        "Korea Standard Time" => "Asia/Seoul",
        "Singapore Standard Time" => "Asia/Singapore",
        "Arabian Standard Time" => "Asia/Dubai",
        "AUS Eastern Standard Time" => "Australia/Sydney",
        "New Zealand Standard Time" => "Pacific/Auckland",
        "South Africa Standard Time" => "Africa/Johannesburg",
        "Morocco Standard Time" => "Africa/Casablanca",
        "Canary Standard Time" => "Atlantic/Canary",
        _ => return None,
    })
}

pub fn resolve_tz(tzid: &str) -> Option<Tz> {
    let tzid = tzid.trim().trim_matches('"');
    if let Ok(tz) = tzid.parse::<Tz>() {
        return Some(tz);
    }
    if let Some(name) = windows_tz(tzid) {
        return name.parse().ok();
    }
    // Prefijos tipo "/mozilla.org/20050126_1/Europe/Berlin".
    let parts: Vec<&str> = tzid.split('/').filter(|s| !s.is_empty()).collect();
    for i in 0..parts.len() {
        if let Ok(tz) = parts[i..].join("/").parse::<Tz>() {
            return Some(tz);
        }
    }
    None
}

pub fn parse_time_value(value: &str, tzid: Option<&str>, is_date: bool) -> Option<ITime> {
    let v = value.trim();
    if is_date || (v.len() == 8 && !v.contains('T')) {
        return NaiveDate::parse_from_str(&v[..8.min(v.len())], "%Y%m%d")
            .ok()
            .map(ITime::Date);
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        return NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S")
            .ok()
            .map(|n| ITime::Utc(n.and_utc()));
    }
    let naive = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    match tzid.and_then(resolve_tz) {
        Some(tz) => Some(ITime::Zoned(naive, tz)),
        None => Some(ITime::Floating(naive)),
    }
}

pub fn prop_time(p: &Prop) -> Option<ITime> {
    let is_date = p
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
    parse_time_value(&p.value, p.param("TZID"), is_date)
}

/// Lista separada por comas (EXDATE/RDATE).
fn prop_times(p: &Prop) -> Vec<ITime> {
    let is_date = p
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
    p.value
        .split(',')
        .filter_map(|v| parse_time_value(v, p.param("TZID"), is_date))
        .collect()
}

/// Duración ISO-8601 de iCalendar (`P1D`, `PT1H30M`, `-PT15M`, `P2W`).
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (unit, in_time) {
                    ('W', _) => Duration::weeks(n),
                    ('D', _) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if neg { -total } else { total })
}

// ---------- Eventos ----------

#[derive(Clone, Debug)]
pub struct VEvent {
    pub uid: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub url: Option<String>,
    pub start: ITime,
    pub duration: Duration,
    pub rrule: Option<String>,
    pub rdates: Vec<ITime>,
    pub exdates: Vec<ITime>,
    pub recurrence_id: Option<ITime>,
    pub cancelled: bool,
}

fn text_prop(c: &Component, name: &str) -> Option<String> {
    c.prop(name)
        .map(|p| unescape(&p.value).trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn vevent_from(c: &Component) -> Option<VEvent> {
    let start = prop_time(c.prop("DTSTART")?)?;
    let end = c.prop("DTEND").and_then(prop_time);
    let duration = match (
        &end,
        c.prop("DURATION").and_then(|p| parse_duration(&p.value)),
    ) {
        (Some(e), _) => e.to_utc() - start.to_utc(),
        (None, Some(d)) => d,
        (None, None) if start.is_date() => Duration::days(1),
        _ => Duration::zero(),
    };
    let duration = if duration < Duration::zero() {
        Duration::zero()
    } else {
        duration
    };
    Some(VEvent {
        uid: text_prop(c, "UID").unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        summary: text_prop(c, "SUMMARY").unwrap_or_else(|| "(Sin título)".into()),
        description: text_prop(c, "DESCRIPTION"),
        location: text_prop(c, "LOCATION"),
        url: text_prop(c, "URL"),
        start,
        duration,
        rrule: c.prop("RRULE").map(|p| p.value.clone()),
        rdates: c.props_named("RDATE").flat_map(prop_times).collect(),
        exdates: c.props_named("EXDATE").flat_map(prop_times).collect(),
        recurrence_id: c.prop("RECURRENCE-ID").and_then(prop_time),
        cancelled: c
            .prop("STATUS")
            .is_some_and(|p| p.value.eq_ignore_ascii_case("CANCELLED")),
    })
}

/// Todos los VEVENT del documento (recorre VCALENDARs anidados).
pub fn vevents(text: &str) -> Vec<VEvent> {
    fn walk(c: &Component, out: &mut Vec<VEvent>) {
        if c.name == "VEVENT" {
            if let Some(ev) = vevent_from(c) {
                out.push(ev);
            }
        }
        for ch in &c.children {
            walk(ch, out);
        }
    }
    let mut out = Vec::new();
    for root in parse(text) {
        walk(&root, &mut out);
    }
    out
}

/// Nombre del calendario en feeds ICS (`X-WR-CALNAME`).
pub fn calendar_name(text: &str) -> Option<String> {
    parse(text)
        .into_iter()
        .find_map(|c| text_prop(&c, "X-WR-CALNAME"))
}

fn fmt_utc(d: &DateTime<Utc>) -> String {
    d.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Normaliza UNTIL para que sea UTC, como exige el crate `rrule`.
fn normalize_rrule(rule: &str) -> String {
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("UNTIL") => {
                let v = v.trim();
                if v.len() == 8 {
                    format!("UNTIL={v}T235959Z")
                } else if !v.ends_with('Z') {
                    format!("UNTIL={v}Z")
                } else {
                    format!("UNTIL={v}")
                }
            }
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Inicios (UTC) de las ocurrencias de un evento recurrente dentro del rango.
fn occurrences(ev: &VEvent, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let dtstart = match &ev.start {
        ITime::Zoned(n, tz) => format!("DTSTART;TZID={}:{}", tz.name(), n.format("%Y%m%dT%H%M%S")),
        other => format!("DTSTART:{}", fmt_utc(&other.to_utc())),
    };
    let mut lines = vec![dtstart];
    if let Some(r) = &ev.rrule {
        lines.push(format!("RRULE:{}", normalize_rrule(r)));
    }
    for r in &ev.rdates {
        lines.push(format!("RDATE:{}", fmt_utc(&r.to_utc())));
    }
    for x in &ev.exdates {
        lines.push(format!("EXDATE:{}", fmt_utc(&x.to_utc())));
    }
    let text = lines.join("\n");
    match text.parse::<rrule::RRuleSet>() {
        Ok(set) => {
            let after = (from - ev.duration - Duration::seconds(1)).with_timezone(&rrule::Tz::UTC);
            let before = to.with_timezone(&rrule::Tz::UTC);
            set.after(after)
                .before(before)
                .all(2000)
                .dates
                .into_iter()
                .map(|d| d.with_timezone(&Utc))
                .collect()
        }
        Err(e) => {
            log::warn!("RRULE no soportada para {} ({e}): {text}", ev.uid);
            vec![ev.start.to_utc()]
        }
    }
}

fn overlaps(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> bool {
    start < to && (end > from || start >= from)
}

/// Expande los eventos al rango dado. `extra` se adjunta a cada ocurrencia.
pub fn expand(
    events: &[VEvent],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    extra: Option<String>,
) -> Vec<RemoteEvent> {
    // Excepciones (RECURRENCE-ID) agrupadas por UID.
    let mut overrides: HashMap<&str, Vec<&VEvent>> = HashMap::new();
    for ev in events.iter().filter(|e| e.recurrence_id.is_some()) {
        overrides.entry(ev.uid.as_str()).or_default().push(ev);
    }
    let mut out = Vec::new();
    let make = |ev: &VEvent, start: DateTime<Utc>, recurring: bool| -> RemoteEvent {
        let all_day = ev.start.is_date();
        let remote_id = if recurring {
            format!("{}#{}", ev.uid, fmt_utc(&start))
        } else {
            ev.uid.clone()
        };
        RemoteEvent {
            remote_id,
            title: ev.summary.clone(),
            description: ev.description.clone(),
            location: ev.location.clone(),
            all_day,
            start,
            end: start + ev.duration,
            etag: None,
            url: ev.url.clone(),
            recurring,
            extra: extra.clone(),
        }
    };

    for ev in events.iter().filter(|e| e.recurrence_id.is_none()) {
        if ev.cancelled {
            continue;
        }
        let is_recurring = ev.rrule.is_some() || !ev.rdates.is_empty();
        if !is_recurring {
            let s = ev.start.to_utc();
            if overlaps(s, s + ev.duration, from, to) {
                out.push(make(ev, s, false));
            }
            continue;
        }
        let replaced: Vec<DateTime<Utc>> = overrides
            .get(ev.uid.as_str())
            .map(|v| {
                v.iter()
                    .filter_map(|o| o.recurrence_id.as_ref().map(ITime::to_utc))
                    .collect()
            })
            .unwrap_or_default();
        for s in occurrences(ev, from, to) {
            if replaced.contains(&s) || !overlaps(s, s + ev.duration, from, to) {
                continue;
            }
            out.push(make(ev, s, true));
        }
    }
    // Ocurrencias modificadas.
    for list in overrides.values() {
        for o in list {
            if o.cancelled {
                continue;
            }
            let s = o.start.to_utc();
            if overlaps(s, s + o.duration, from, to) {
                let mut e = make(o, s, true);
                e.remote_id = format!(
                    "{}#{}",
                    o.uid,
                    fmt_utc(&o.recurrence_id.as_ref().unwrap().to_utc())
                );
                out.push(e);
            }
        }
    }
    out
}

// ---------- Escritura ----------

/// Pliega una línea a 75 octetos sin partir caracteres UTF-8.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for ch in line.chars() {
        let len = ch.len_utf8();
        if count + len > 75 {
            out.push_str("\r\n ");
            count = 1;
        }
        out.push(ch);
        count += len;
    }
    out
}

fn time_lines(ev: &NewEvent) -> [String; 2] {
    if ev.all_day {
        [
            format!("DTSTART;VALUE=DATE:{}", ev.start.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", ev.end.format("%Y%m%d")),
        ]
    } else {
        [
            format!("DTSTART:{}", fmt_utc(&ev.start)),
            format!("DTEND:{}", fmt_utc(&ev.end)),
        ]
    }
}

fn content_lines(ev: &NewEvent) -> Vec<String> {
    let mut lines = Vec::new();
    lines.extend(time_lines(ev));
    lines.push(format!("SUMMARY:{}", escape(&ev.title)));
    if let Some(l) = &ev.location {
        lines.push(format!("LOCATION:{}", escape(l)));
    }
    if let Some(d) = &ev.description {
        lines.push(format!("DESCRIPTION:{}", escape(d)));
    }
    lines.push(format!("DTSTAMP:{}", fmt_utc(&Utc::now())));
    lines
}

pub fn build_calendar(uid: &str, ev: &NewEvent) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//FERSOFT//Kandy//ES".into(),
        "CALSCALE:GREGORIAN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{uid}"),
    ];
    lines.extend(content_lines(ev));
    lines.push("END:VEVENT".into());
    lines.push("END:VCALENDAR".into());
    lines
        .iter()
        .map(|l| fold(l))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

/// Modifica el VEVENT principal de un recurso existente conservando el resto
/// de propiedades (asistentes, alarmas, etc.).
pub fn patch_calendar(original: &str, ev: &NewEvent) -> String {
    const REPLACED: [&str; 8] = [
        "DTSTART",
        "DTEND",
        "DURATION",
        "SUMMARY",
        "LOCATION",
        "DESCRIPTION",
        "DTSTAMP",
        "SEQUENCE",
    ];
    let lines = unfold(original);
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    let mut depth_event = false;
    let mut is_master = true;
    let mut seq = 0i64;
    let mut buffer: Vec<String> = Vec::new();
    let mut patched = false;
    for line in lines {
        let p = parse_line(&line);
        let name = p.as_ref().map(|p| p.name.as_str()).unwrap_or("");
        let value = p.as_ref().map(|p| p.value.trim().to_ascii_uppercase());
        if name == "BEGIN" && value.as_deref() == Some("VEVENT") && !patched {
            depth_event = true;
            is_master = true;
            buffer.clear();
            buffer.push(line);
            continue;
        }
        if depth_event {
            if name == "RECURRENCE-ID" {
                is_master = false;
            }
            if name == "END" && value.as_deref() == Some("VEVENT") {
                if is_master {
                    buffer.retain(|l| {
                        let n = parse_line(l).map(|p| p.name).unwrap_or_default();
                        if n == "SEQUENCE" {
                            seq = l
                                .rsplit(':')
                                .next()
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0);
                        }
                        !REPLACED.contains(&n.as_str())
                    });
                    buffer.extend(content_lines(ev));
                    buffer.push(format!("SEQUENCE:{}", seq + 1));
                    patched = true;
                }
                buffer.push(line);
                out.append(&mut buffer);
                depth_event = false;
                continue;
            }
            buffer.push(line);
            continue;
        }
        out.push(line);
    }
    out.iter().map(|l| fold(l)).collect::<Vec<_>>().join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Prueba\r\n\
BEGIN:VEVENT\r\nUID:abc\r\nDTSTART;TZID=Europe/Madrid:20240101T100000\r\n\
DTEND;TZID=Europe/Madrid:20240101T110000\r\nRRULE:FREQ=WEEKLY;COUNT=5\r\n\
EXDATE;TZID=Europe/Madrid:20240108T100000\r\nSUMMARY:Reuni\r\n ón\\, semanal\r\n\
ATTENDEE;CN=\"Doe: John\":mailto:j@x.com\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:abc\r\nRECURRENCE-ID;TZID=Europe/Madrid:20240115T100000\r\n\
DTSTART;TZID=Europe/Madrid:20240115T120000\r\nDTEND;TZID=Europe/Madrid:20240115T130000\r\n\
SUMMARY:Movida\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:allday\r\nDTSTART;VALUE=DATE:20240103\r\nSUMMARY:Fiesta\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_and_expands() {
        let evs = vevents(SAMPLE);
        assert_eq!(evs.len(), 3);
        assert_eq!(evs[0].summary, "Reunión, semanal");
        assert_eq!(calendar_name(SAMPLE).as_deref(), Some("Prueba"));
        let from = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2024, 3, 1, 0, 0, 0).unwrap();
        let mut out = expand(&evs, from, to, None);
        out.sort_by_key(|e| e.start);
        let titles: Vec<_> = out
            .iter()
            .map(|e| (e.title.as_str(), e.start.to_rfc3339()))
            .collect();
        // 5 ocurrencias - 1 EXDATE - 1 movida + 1 excepción + 1 de día completo = 5
        assert_eq!(out.len(), 5, "{titles:?}");
        assert_eq!(
            out[0].start,
            Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap()
        );
        assert!(out.iter().any(|e| e.title == "Movida"
            && e.start == Utc.with_ymd_and_hms(2024, 1, 15, 11, 0, 0).unwrap()));
        let fiesta = out.iter().find(|e| e.title == "Fiesta").unwrap();
        assert!(fiesta.all_day);
        assert_eq!(fiesta.end - fiesta.start, Duration::days(1));
    }

    #[test]
    fn rrule_with_local_until() {
        let ics =
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u\nDTSTART;TZID=America/Bogota:20240101T090000\n\
DURATION:PT30M\nRRULE:FREQ=DAILY;UNTIL=20240105T090000\nSUMMARY:x\nEND:VEVENT\nEND:VCALENDAR\n";
        let evs = vevents(ics);
        let from = Utc.with_ymd_and_hms(2023, 12, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();
        let out = expand(&evs, from, to, None);
        assert!(out.len() >= 4, "{}", out.len());
        assert_eq!(out[0].end - out[0].start, Duration::minutes(30));
    }

    #[test]
    fn windows_and_prefixed_tz() {
        assert_eq!(
            resolve_tz("Romance Standard Time"),
            Some(chrono_tz::Europe::Paris)
        );
        assert_eq!(
            resolve_tz("/mozilla.org/20050126_1/Europe/Berlin"),
            Some(chrono_tz::Europe::Berlin)
        );
        assert_eq!(parse_duration("-PT15M"), Some(-Duration::minutes(15)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
    }

    #[test]
    fn build_and_patch_roundtrip() {
        let ev = NewEvent {
            title: "Café; con, amigos".into(),
            description: Some("Línea 1\nLínea 2".into()),
            location: None,
            all_day: false,
            start: Utc.with_ymd_and_hms(2024, 5, 1, 8, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2024, 5, 1, 9, 0, 0).unwrap(),
        };
        let ics = build_calendar("id-1", &ev);
        let parsed = vevents(&ics);
        assert_eq!(parsed[0].summary, "Café; con, amigos");
        assert_eq!(parsed[0].description.as_deref(), Some("Línea 1\nLínea 2"));

        let mut ev2 = ev.clone();
        ev2.title = "Nuevo".into();
        ev2.all_day = true;
        ev2.start = Utc.with_ymd_and_hms(2024, 5, 2, 0, 0, 0).unwrap();
        ev2.end = Utc.with_ymd_and_hms(2024, 5, 3, 0, 0, 0).unwrap();
        let patched = patch_calendar(
            &(SAMPLE.replace("SUMMARY:Fiesta", "SUMMARY:Fiesta\r\nSEQUENCE:3")),
            &ev2,
        );
        let evs = vevents(&patched);
        // El master (primer VEVENT) cambia; la excepción se conserva intacta.
        assert_eq!(evs[0].summary, "Nuevo");
        assert!(evs[0].start.is_date());
        assert_eq!(evs[1].summary, "Movida");
        assert!(patched.contains("ATTENDEE;CN=\"Doe: John\":mailto:j@x.com"));
        assert!(patched.lines().all(|l| l.len() <= 76));
    }
}
