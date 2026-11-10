// Contrato tipado con los comandos Rust (src-tauri/src/commands.rs).
import { invoke } from "@tauri-apps/api/core";

export type ProviderKind = "google" | "microsoft" | "zoho" | "caldav" | "ics";

export interface CalendarInfo {
  id: string;
  accountId: string;
  name: string;
  color: string;
  readOnly: boolean;
  visible: boolean;
}

export interface AccountInfo {
  id: string;
  provider: ProviderKind;
  name: string;
  email: string | null;
  lastSync: string | null;
  lastError: string | null;
  calendars: CalendarInfo[];
}

export interface CalendarEvent {
  id: string;
  calendarId: string;
  title: string;
  description: string | null;
  location: string | null;
  /** ISO UTC para eventos con hora, `YYYY-MM-DD` para día completo (fin exclusivo). */
  start: string;
  end: string;
  allDay: boolean;
  url: string | null;
  readOnly: boolean;
  recurring: boolean;
  color: string;
}

export interface EventDraft {
  title: string;
  description: string | null;
  location: string | null;
  start: string;
  end: string;
  allDay: boolean;
}

export type EventDialogMode =
  | { kind: "create"; start: Date; end: Date; allDay: boolean }
  | { kind: "edit"; event: CalendarEvent };

export interface Settings {
  googleClientId: string;
  googleClientSecret: string;
  microsoftClientId: string;
  microsoftTenant: string;
  zohoClientId: string;
  zohoClientSecret: string;
  zohoRedirectPort: number;
  syncIntervalMinutes: number;
  reminderMinutes: number;
}

export interface SettingsPayload {
  settings: Settings;
  builtIn: { google: boolean; microsoft: boolean; zoho: boolean };
}

export interface SyncProgress {
  accountId: string;
  status: "syncing" | "ok" | "error";
  message: string | null;
}

export const api = {
  listAccounts: () => invoke<AccountInfo[]>("list_accounts"),
  setCalendarVisibility: (calendarId: string, visible: boolean) =>
    invoke<void>("set_calendar_visibility", { calendarId, visible }),
  setCalendarColor: (calendarId: string, color: string) =>
    invoke<void>("set_calendar_color", { calendarId, color }),
  removeAccount: (accountId: string) => invoke<void>("remove_account", { accountId }),

  connectOauth: (provider: ProviderKind, zohoLocation?: string, accountId?: string) =>
    invoke<AccountInfo>("connect_oauth", {
      provider,
      zohoLocation: zohoLocation ?? null,
      accountId: accountId ?? null,
    }),
  cancelOauth: () => invoke<void>("cancel_oauth"),
  connectCaldav: (name: string, url: string, username: string, password: string) =>
    invoke<AccountInfo>("connect_caldav", { name, url, username, password }),
  addIcs: (name: string, url: string, color: string | null) =>
    invoke<AccountInfo>("add_ics", { name, url, color }),

  syncNow: (start: Date, end: Date, accountId?: string) =>
    invoke<void>("sync_now", {
      start: start.toISOString(),
      end: end.toISOString(),
      accountId: accountId ?? null,
    }),

  listEvents: (start: Date, end: Date) =>
    invoke<CalendarEvent[]>("list_events", {
      start: start.toISOString(),
      end: end.toISOString(),
    }),
  createEvent: (calendarId: string, draft: EventDraft) =>
    invoke<CalendarEvent>("create_event", { calendarId, draft }),
  updateEvent: (eventId: string, draft: EventDraft) =>
    invoke<CalendarEvent>("update_event", { eventId, draft }),
  deleteEvent: (eventId: string) => invoke<void>("delete_event", { eventId }),

  getSettings: () => invoke<SettingsPayload>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
};

export const PROVIDERS: Record<ProviderKind, { label: string; color: string }> = {
  google: { label: "Google", color: "#4285f4" },
  microsoft: { label: "Microsoft", color: "#0078d4" },
  zoho: { label: "Zoho", color: "#e42527" },
  caldav: { label: "CalDAV", color: "#6b7280" },
  ics: { label: "Suscripción ICS", color: "#8b5cf6" },
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
