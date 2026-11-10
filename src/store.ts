// Estado global mínimo (sin Pinia): cuentas, estado de sincronización y avisos.
import { reactive } from "vue";
import { api, errorText, type AccountInfo } from "./api";

export interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  text: string;
}

export const store = reactive({
  accounts: [] as AccountInfo[],
  syncing: new Set<string>(),
  toasts: [] as Toast[],
});

let toastSeq = 0;

export function toast(text: string, kind: Toast["kind"] = "info", ms = 5000) {
  const id = ++toastSeq;
  store.toasts.push({ id, kind, text });
  setTimeout(() => dismiss(id), kind === "error" ? Math.max(ms, 8000) : ms);
}

export function dismiss(id: number) {
  const i = store.toasts.findIndex((t) => t.id === id);
  if (i >= 0) store.toasts.splice(i, 1);
}

export function toastError(prefix: string, e: unknown) {
  toast(`${prefix}: ${errorText(e)}`, "error");
}

export async function refreshAccounts() {
  try {
    store.accounts = await api.listAccounts();
  } catch (e) {
    toastError("No se pudieron cargar las cuentas", e);
  }
}

export function calendarById(id: string) {
  for (const a of store.accounts) {
    const c = a.calendars.find((c) => c.id === id);
    if (c) return { account: a, calendar: c };
  }
  return null;
}

export function writableCalendars() {
  return store.accounts.flatMap((a) =>
    a.calendars.filter((c) => !c.readOnly).map((c) => ({ account: a, calendar: c })),
  );
}
