//! Caché local en SQLite: cuentas, calendarios, eventos y ajustes.
//! La UI siempre lee de aquí, así la app funciona sin conexión.

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{fail, Result};
use crate::models::*;
use crate::settings::Settings;

pub struct Db {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS accounts (
    id          TEXT PRIMARY KEY,
    provider    TEXT NOT NULL,
    name        TEXT NOT NULL,
    email       TEXT,
    config      TEXT NOT NULL DEFAULT '{}',
    last_sync   TEXT,
    last_error  TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS calendars (
    id              TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id       TEXT NOT NULL,
    name            TEXT NOT NULL,
    color           TEXT NOT NULL,
    color_override  INTEGER NOT NULL DEFAULT 0,
    read_only       INTEGER NOT NULL DEFAULT 0,
    visible         INTEGER NOT NULL DEFAULT 1,
    is_primary      INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account_id, remote_id)
);

CREATE TABLE IF NOT EXISTS events (
    id           TEXT PRIMARY KEY,
    calendar_id  TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    remote_id    TEXT NOT NULL,
    title        TEXT NOT NULL,
    description  TEXT,
    location     TEXT,
    all_day      INTEGER NOT NULL,
    start_ts     INTEGER NOT NULL,
    end_ts       INTEGER NOT NULL,
    etag         TEXT,
    url          TEXT,
    recurring    INTEGER NOT NULL DEFAULT 0,
    extra        TEXT,
    UNIQUE(calendar_id, remote_id)
);
CREATE INDEX IF NOT EXISTS idx_events_range ON events(start_ts, end_ts);
CREATE INDEX IF NOT EXISTS idx_events_cal ON events(calendar_id, start_ts);

CREATE TABLE IF NOT EXISTS settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
"#;

/// Identificador local determinista de un evento.
pub fn event_id(calendar_id: &str, remote_id: &str) -> String {
    format!("{calendar_id}::{remote_id}")
}

fn ts(dt: &DateTime<Utc>) -> i64 {
    dt.timestamp()
}

fn from_ts(t: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(t, 0).single().unwrap_or_default()
}

fn row_account(r: &Row) -> rusqlite::Result<Account> {
    let provider: String = r.get("provider")?;
    let config: String = r.get("config")?;
    Ok(Account {
        id: r.get("id")?,
        provider: ProviderKind::parse(&provider).unwrap_or(ProviderKind::Ics),
        name: r.get("name")?,
        email: r.get("email")?,
        config: serde_json::from_str(&config).unwrap_or_default(),
        last_sync: r.get("last_sync")?,
        last_error: r.get("last_error")?,
    })
}

fn row_calendar(r: &Row) -> rusqlite::Result<Calendar> {
    Ok(Calendar {
        id: r.get("id")?,
        account_id: r.get("account_id")?,
        remote_id: r.get("remote_id")?,
        name: r.get("name")?,
        color: r.get("color")?,
        read_only: r.get("read_only")?,
        visible: r.get("visible")?,
    })
}

fn row_event(r: &Row) -> rusqlite::Result<StoredEvent> {
    Ok(StoredEvent {
        id: r.get("id")?,
        calendar_id: r.get("calendar_id")?,
        ev: RemoteEvent {
            remote_id: r.get("remote_id")?,
            title: r.get("title")?,
            description: r.get("description")?,
            location: r.get("location")?,
            all_day: r.get("all_day")?,
            start: from_ts(r.get("start_ts")?),
            end: from_ts(r.get("end_ts")?),
            etag: r.get("etag")?,
            url: r.get("url")?,
            recurring: r.get("recurring")?,
            extra: r.get("extra")?,
        },
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // Si un hilo entró en pánico con el lock tomado, seguimos con la conexión.
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ---------- Cuentas ----------

    pub fn insert_account(&self, a: &Account) -> Result<()> {
        self.conn().execute(
            "INSERT INTO accounts (id, provider, name, email, config, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                a.id,
                a.provider.as_str(),
                a.name,
                a.email,
                serde_json::to_string(&a.config)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn update_account_identity(&self, a: &Account) -> Result<()> {
        self.conn().execute(
            "UPDATE accounts SET name = ?2, email = ?3, config = ?4 WHERE id = ?1",
            params![a.id, a.name, a.email, serde_json::to_string(&a.config)?],
        )?;
        Ok(())
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.conn();
        let mut st = conn.prepare("SELECT * FROM accounts ORDER BY created_at")?;
        let rows = st.query_map([], row_account)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_account(&self, id: &str) -> Result<Account> {
        match self
            .conn()
            .query_row("SELECT * FROM accounts WHERE id = ?1", [id], row_account)
            .optional()?
        {
            Some(a) => Ok(a),
            None => fail("La cuenta no existe"),
        }
    }

    pub fn find_account(&self, provider: ProviderKind, email: &str) -> Result<Option<Account>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT * FROM accounts WHERE provider = ?1 AND lower(email) = lower(?2)",
                params![provider.as_str(), email],
                row_account,
            )
            .optional()?)
    }

    pub fn set_account_status(&self, id: &str, ok: bool, error: Option<&str>) -> Result<()> {
        let conn = self.conn();
        if ok {
            conn.execute(
                "UPDATE accounts SET last_sync = ?2, last_error = NULL WHERE id = ?1",
                params![id, Utc::now().to_rfc3339()],
            )?;
        } else {
            conn.execute(
                "UPDATE accounts SET last_error = ?2 WHERE id = ?1",
                params![id, error],
            )?;
        }
        Ok(())
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---------- Calendarios ----------

    /// Inserta/actualiza los calendarios remotos y elimina los que ya no existen.
    /// Respeta la visibilidad y el color que el usuario haya cambiado.
    pub fn upsert_calendars(&self, account_id: &str, remote: &[RemoteCalendar]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let existing: i64 = tx.query_row(
            "SELECT COUNT(*) FROM calendars WHERE account_id = ?1",
            [account_id],
            |r| r.get(0),
        )?;
        for (i, rc) in remote.iter().enumerate() {
            let color = rc
                .color
                .as_deref()
                .and_then(normalize_color)
                .unwrap_or_else(|| PALETTE[(existing as usize + i) % PALETTE.len()].to_string());
            tx.execute(
                "INSERT INTO calendars (id, account_id, remote_id, name, color, read_only, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                    name = excluded.name,
                    read_only = excluded.read_only,
                    is_primary = excluded.is_primary,
                    color = CASE WHEN calendars.color_override = 1 THEN calendars.color ELSE excluded.color END",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    account_id,
                    rc.remote_id,
                    rc.name,
                    color,
                    rc.read_only,
                    rc.primary
                ],
            )?;
        }
        // Borrar calendarios que desaparecieron en el servidor.
        let keep: Vec<&str> = remote.iter().map(|r| r.remote_id.as_str()).collect();
        let current: Vec<(String, String)> = {
            let mut st = tx.prepare("SELECT id, remote_id FROM calendars WHERE account_id = ?1")?;
            let rows = st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (id, rid) in current {
            if !keep.contains(&rid.as_str()) {
                tx.execute("DELETE FROM calendars WHERE id = ?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_calendars(&self, account_id: Option<&str>) -> Result<Vec<Calendar>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT * FROM calendars WHERE (?1 IS NULL OR account_id = ?1)
             ORDER BY is_primary DESC, name COLLATE NOCASE",
        )?;
        let rows = st.query_map([account_id], row_calendar)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_calendar(&self, id: &str) -> Result<Calendar> {
        match self
            .conn()
            .query_row("SELECT * FROM calendars WHERE id = ?1", [id], row_calendar)
            .optional()?
        {
            Some(c) => Ok(c),
            None => fail("El calendario no existe"),
        }
    }

    pub fn set_calendar_visible(&self, id: &str, visible: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE calendars SET visible = ?2 WHERE id = ?1",
            params![id, visible],
        )?;
        Ok(())
    }

    pub fn set_calendar_color(&self, id: &str, color: &str) -> Result<()> {
        let Some(color) = normalize_color(color) else {
            return fail("Color inválido");
        };
        self.conn().execute(
            "UPDATE calendars SET color = ?2, color_override = 1 WHERE id = ?1",
            params![id, color],
        )?;
        Ok(())
    }

    // ---------- Eventos ----------

    /// Sustituye los eventos de un calendario dentro de un rango por los recibidos.
    pub fn replace_events(
        &self,
        calendar_id: &str,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        events: &[RemoteEvent],
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM events WHERE calendar_id = ?1 AND start_ts < ?3
               AND (end_ts > ?2 OR start_ts >= ?2)",
            params![calendar_id, ts(start), ts(end)],
        )?;
        for ev in events {
            Self::put_event(&tx, calendar_id, ev)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn put_event(conn: &Connection, calendar_id: &str, ev: &RemoteEvent) -> Result<String> {
        let id = event_id(calendar_id, &ev.remote_id);
        conn.execute(
            "INSERT OR REPLACE INTO events
               (id, calendar_id, remote_id, title, description, location, all_day,
                start_ts, end_ts, etag, url, recurring, extra)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                calendar_id,
                ev.remote_id,
                ev.title,
                ev.description,
                ev.location,
                ev.all_day,
                ts(&ev.start),
                ts(&ev.end),
                ev.etag,
                ev.url,
                ev.recurring,
                ev.extra
            ],
        )?;
        Ok(id)
    }

    pub fn upsert_event(&self, calendar_id: &str, ev: &RemoteEvent) -> Result<StoredEvent> {
        let id = Self::put_event(&self.conn(), calendar_id, ev)?;
        Ok(StoredEvent {
            id,
            calendar_id: calendar_id.to_string(),
            ev: ev.clone(),
        })
    }

    pub fn get_event(&self, id: &str) -> Result<StoredEvent> {
        match self
            .conn()
            .query_row("SELECT * FROM events WHERE id = ?1", [id], row_event)
            .optional()?
        {
            Some(e) => Ok(e),
            None => fail("El evento ya no existe; sincroniza de nuevo"),
        }
    }

    pub fn delete_event(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM events WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Eventos de calendarios visibles que se solapan con el rango.
    pub fn list_events(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Vec<(StoredEvent, Calendar, ProviderKind)>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT e.*, c.id AS c_id, c.account_id, c.remote_id AS c_remote_id, c.name AS c_name,
                    c.color, c.read_only, c.visible, a.provider
             FROM events e
             JOIN calendars c ON c.id = e.calendar_id
             JOIN accounts a ON a.id = c.account_id
             WHERE c.visible = 1 AND e.start_ts < ?2 AND (e.end_ts > ?1 OR e.start_ts >= ?1)
             ORDER BY e.start_ts",
        )?;
        let rows = st.query_map(params![ts(start), ts(end)], |r| {
            let ev = row_event(r)?;
            let cal = Calendar {
                id: r.get("c_id")?,
                account_id: r.get("account_id")?,
                remote_id: r.get("c_remote_id")?,
                name: r.get("c_name")?,
                color: r.get("color")?,
                read_only: r.get("read_only")?,
                visible: r.get("visible")?,
            };
            let p: String = r.get("provider")?;
            Ok((
                ev,
                cal,
                ProviderKind::parse(&p).unwrap_or(ProviderKind::Ics),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Eventos con hora que empiezan en el intervalo (para recordatorios).
    pub fn upcoming_events(
        &self,
        from: &DateTime<Utc>,
        to: &DateTime<Utc>,
    ) -> Result<Vec<(StoredEvent, String)>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT e.*, c.name AS c_name FROM events e
             JOIN calendars c ON c.id = e.calendar_id
             WHERE c.visible = 1 AND e.all_day = 0 AND e.start_ts >= ?1 AND e.start_ts < ?2",
        )?;
        let rows = st.query_map(params![ts(from), ts(to)], |r| {
            Ok((row_event(r)?, r.get::<_, String>("c_name")?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---------- Ajustes ----------

    pub fn settings(&self) -> Result<Settings> {
        let raw: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = 'app'", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES ('app', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(s)?],
        )?;
        Ok(())
    }
}
