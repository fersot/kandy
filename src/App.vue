<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import FullCalendar from "@fullcalendar/vue3";
import dayGridPlugin from "@fullcalendar/daygrid";
import timeGridPlugin from "@fullcalendar/timegrid";
import listPlugin from "@fullcalendar/list";
import interactionPlugin from "@fullcalendar/interaction";
import esLocale from "@fullcalendar/core/locales/es";
import type {
  CalendarOptions,
  DatesSetArg,
  DateSelectArg,
  EventClickArg,
  EventDropArg,
  EventInput,
  EventSourceFuncArg,
} from "@fullcalendar/core";
import type { EventResizeDoneArg } from "@fullcalendar/interaction";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  api,
  type AccountInfo,
  type CalendarEvent,
  type CalendarInfo,
  type EventDialogMode,
  type SyncProgress,
} from "./api";
import { refreshAccounts, store, toast, toastError, writableCalendars } from "./store";
import { addDays, contrastText, dateStr, nextHour } from "./dates";
import Sidebar from "./components/Sidebar.vue";
import EventDialog from "./components/EventDialog.vue";
import AddAccountDialog from "./components/AddAccountDialog.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import Toasts from "./components/Toasts.vue";

const VIEW_KEY = "kandy.view";
const calendarRef = ref<InstanceType<typeof FullCalendar> | null>(null);
const eventDialog = ref<EventDialogMode | null>(null);
const showAdd = ref(false);
const reconnectAccount = ref<AccountInfo | null>(null);
const showSettings = ref(false);

// ---------- Sincronización por rangos ----------
// Se sincroniza la ventana visible ±1 mes y se recuerda lo cubierto para no
// repetir peticiones al navegar; el backend refresca además cada N minutos.
let covered: Array<[number, number]> = [];
let visibleRange: { start: Date; end: Date } | null = null;

const isCovered = (s: Date, e: Date) => covered.some(([a, b]) => a <= s.getTime() && b >= e.getTime());

function windowAround(start: Date, end: Date): [Date, Date] {
  return [addDays(start, -31), addDays(end, 31)];
}

function refetch() {
  calendarRef.value?.getApi().refetchEvents();
}

async function syncRange(start: Date, end: Date, accountId?: string) {
  try {
    await api.syncNow(start, end, accountId);
    if (!accountId) covered.push([start.getTime(), end.getTime()]);
  } catch (e) {
    toastError("Error al sincronizar", e);
  }
  await refreshAccounts();
  refetch();
}

function ensureSynced() {
  if (!visibleRange || !store.accounts.length) return;
  if (isCovered(visibleRange.start, visibleRange.end)) return;
  const [s, e] = windowAround(visibleRange.start, visibleRange.end);
  syncRange(s, e);
}

function syncVisible(accountId?: string) {
  if (!visibleRange) return;
  if (!accountId) covered = [];
  const [s, e] = windowAround(visibleRange.start, visibleRange.end);
  syncRange(s, e, accountId);
}

// ---------- FullCalendar ----------
function toInput(e: CalendarEvent): EventInput {
  return {
    id: e.id,
    title: e.title,
    start: e.start,
    end: e.end,
    allDay: e.allDay,
    backgroundColor: e.color,
    borderColor: e.color,
    textColor: contrastText(e.color),
    editable: !e.readOnly,
    classNames: e.readOnly ? ["ro"] : [],
    extendedProps: { data: e },
  };
}

async function fetchEvents(
  info: EventSourceFuncArg,
  success: (events: EventInput[]) => void,
  failure: (error: Error) => void,
) {
  try {
    const events = await api.listEvents(info.start, info.end);
    success(events.map(toInput));
  } catch (e) {
    failure(e instanceof Error ? e : new Error(String(e)));
    toastError("No se pudieron cargar los eventos", e);
  }
}

function onDatesSet(arg: DatesSetArg) {
  visibleRange = { start: arg.start, end: arg.end };
  try {
    localStorage.setItem(VIEW_KEY, arg.view.type);
  } catch {
    /* ignorar */
  }
  ensureSynced();
}

function openCreate(start: Date, end: Date, allDay: boolean) {
  if (!writableCalendars().length) {
    toast(
      store.accounts.length
        ? "No tienes calendarios con permiso de escritura"
        : "Conecta primero una cuenta de calendario",
    );
    if (!store.accounts.length) showAdd.value = true;
    return;
  }
  eventDialog.value = { kind: "create", start, end, allDay };
}

function onSelect(arg: DateSelectArg) {
  openCreate(arg.start, arg.end, arg.allDay);
  arg.view.calendar.unselect();
}

function onEventClick(arg: EventClickArg) {
  arg.jsEvent.preventDefault();
  eventDialog.value = { kind: "edit", event: arg.event.extendedProps.data as CalendarEvent };
}

async function onEventChange(arg: EventDropArg | EventResizeDoneArg) {
  const data = arg.event.extendedProps.data as CalendarEvent;
  const allDay = arg.event.allDay;
  const start = arg.event.start!;
  const end = arg.event.end ?? (allDay ? addDays(start, 1) : new Date(start.getTime() + 3600_000));
  try {
    await api.updateEvent(data.id, {
      title: data.title,
      description: data.description,
      location: data.location,
      allDay,
      start: allDay ? dateStr(start) : start.toISOString(),
      end: allDay ? dateStr(end) : end.toISOString(),
    });
    toast("Evento actualizado", "success", 2000);
    refetch();
  } catch (e) {
    arg.revert();
    toastError("No se pudo mover el evento", e);
  }
}

function initialView(): string {
  try {
    return localStorage.getItem(VIEW_KEY) || "timeGridWeek";
  } catch {
    return "timeGridWeek";
  }
}

const calendarOptions: CalendarOptions = {
  plugins: [dayGridPlugin, timeGridPlugin, listPlugin, interactionPlugin],
  locale: esLocale,
  initialView: initialView(),
  headerToolbar: {
    left: "prev,next today",
    center: "title",
    right: "dayGridMonth,timeGridWeek,timeGridDay,listWeek",
  },
  height: "100%",
  firstDay: 1,
  nowIndicator: true,
  selectable: true,
  selectMirror: true,
  editable: true,
  dayMaxEvents: true,
  navLinks: true,
  weekNumbers: false,
  scrollTime: "07:30:00",
  slotDuration: "00:30:00",
  eventMinHeight: 22,
  eventTimeFormat: { hour: "2-digit", minute: "2-digit", hour12: false },
  slotLabelFormat: { hour: "2-digit", minute: "2-digit", hour12: false },
  events: fetchEvents,
  datesSet: onDatesSet,
  select: onSelect,
  eventClick: onEventClick,
  eventDrop: onEventChange,
  eventResize: onEventChange,
};

// ---------- Acciones de la barra lateral ----------
function newEvent() {
  const s = nextHour();
  openCreate(s, new Date(s.getTime() + 3600_000), false);
}

function onAccountAdded(acc: AccountInfo) {
  refreshAccounts();
  syncVisible(acc.id);
}

function onCalendarsChanged(madeVisible?: CalendarInfo) {
  refetch();
  if (madeVisible) syncVisible(madeVisible.accountId);
}

function onReconnect(acc: AccountInfo) {
  reconnectAccount.value = acc;
  showAdd.value = true;
}

function closeAdd() {
  showAdd.value = false;
  reconnectAccount.value = null;
}

// ---------- Atajos de teclado ----------
function onKey(e: KeyboardEvent) {
  if (eventDialog.value || showAdd.value || showSettings.value) return;
  const t = e.target as HTMLElement;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) return;
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  const cal = calendarRef.value?.getApi();
  switch (e.key) {
    case "n":
      newEvent();
      break;
    case "t":
      cal?.today();
      break;
    case "ArrowLeft":
      cal?.prev();
      break;
    case "ArrowRight":
      cal?.next();
      break;
    case "m":
      cal?.changeView("dayGridMonth");
      break;
    case "w":
      cal?.changeView("timeGridWeek");
      break;
    case "d":
      cal?.changeView("timeGridDay");
      break;
    case "a":
      cal?.changeView("listWeek");
      break;
    default:
      return;
  }
  e.preventDefault();
}

const unlisteners: UnlistenFn[] = [];

onMounted(async () => {
  window.addEventListener("keydown", onKey);
  unlisteners.push(
    await listen<SyncProgress>("sync-progress", ({ payload }) => {
      if (payload.status === "syncing") store.syncing.add(payload.accountId);
      else store.syncing.delete(payload.accountId);
    }),
    await listen("sync-finished", async () => {
      await refreshAccounts();
      refetch();
    }),
  );
  await refreshAccounts();
  refetch();
  ensureSynced();
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  unlisteners.forEach((u) => u());
});
</script>

<template>
  <div class="app">
    <Sidebar
      @new-event="newEvent"
      @add-account="showAdd = true"
      @settings="showSettings = true"
      @sync="syncVisible"
      @reconnect="onReconnect"
      @calendars-changed="onCalendarsChanged"
    />
    <main class="main">
      <FullCalendar ref="calendarRef" :options="calendarOptions" />
    </main>

    <EventDialog v-if="eventDialog" :mode="eventDialog" @close="eventDialog = null" @changed="refetch" />
    <AddAccountDialog
      v-if="showAdd"
      :reconnect="reconnectAccount"
      @close="closeAdd"
      @added="onAccountAdded"
      @open-settings="closeAdd(); showSettings = true"
    />
    <SettingsDialog v-if="showSettings" @close="showSettings = false" />
    <Toasts />
  </div>
</template>

<style scoped>
.app {
  display: flex;
  height: 100%;
}
.main {
  flex: 1;
  min-width: 0;
  padding: 16px 18px 14px;
  background: var(--surface);
  display: flex;
  flex-direction: column;
}
.main > :deep(.fc) {
  flex: 1;
  min-height: 0;
}
</style>
