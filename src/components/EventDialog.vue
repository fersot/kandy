<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, PROVIDERS, type EventDialogMode, type EventDraft } from "../api";
import { calendarById, toast, toastError, writableCalendars } from "../store";
import { addDaysStr, dateStr, timeStr } from "../dates";
import Modal from "./Modal.vue";
import Icon from "./Icon.vue";

const props = defineProps<{ mode: EventDialogMode }>();
const emit = defineEmits<{ close: []; changed: [] }>();

const LAST_CAL_KEY = "kandy.lastCalendar";
const writable = writableCalendars();
const editing = props.mode.kind === "edit" ? props.mode.event : null;
const owner = editing ? calendarById(editing.calendarId) : null;
const readOnly = editing?.readOnly ?? false;

function initialCalendar(): string {
  if (editing) return editing.calendarId;
  let last: string | null = null;
  try {
    last = localStorage.getItem(LAST_CAL_KEY);
  } catch {
    /* almacenamiento no disponible */
  }
  return writable.find((w) => w.calendar.id === last)?.calendar.id ?? writable[0]?.calendar.id ?? "";
}

function initialForm() {
  if (editing) {
    if (editing.allDay) {
      return {
        allDay: true,
        startDate: editing.start,
        endDate: addDaysStr(editing.end, -1) < editing.start ? editing.start : addDaysStr(editing.end, -1),
        startTime: "09:00",
        endTime: "10:00",
      };
    }
    const s = new Date(editing.start);
    const e = new Date(editing.end);
    return { allDay: false, startDate: dateStr(s), endDate: dateStr(e), startTime: timeStr(s), endTime: timeStr(e) };
  }
  const m = props.mode as Extract<EventDialogMode, { kind: "create" }>;
  if (m.allDay) {
    return {
      allDay: true,
      startDate: dateStr(m.start),
      endDate: addDaysStr(dateStr(m.end), -1),
      startTime: "09:00",
      endTime: "10:00",
    };
  }
  return {
    allDay: false,
    startDate: dateStr(m.start),
    endDate: dateStr(m.end),
    startTime: timeStr(m.start),
    endTime: timeStr(m.end),
  };
}

const form = reactive({
  title: editing?.title ?? "",
  calendarId: initialCalendar(),
  location: editing?.location ?? "",
  description: editing?.description ?? "",
  ...initialForm(),
});

const busy = ref(false);
const confirmDelete = ref(false);
const selectedColor = computed(
  () => calendarById(form.calendarId)?.calendar.color ?? "var(--accent)",
);
const title = computed(() =>
  readOnly ? "Detalles del evento" : editing ? "Editar evento" : "Nuevo evento",
);

// Mantiene el fin coherente al mover el inicio.
watch(
  () => form.startDate,
  (v, old) => {
    if (!old || !form.endDate) return;
    const delta = (Date.parse(v) - Date.parse(old)) / 86400000;
    if (!Number.isNaN(delta) && delta !== 0) form.endDate = addDaysStr(form.endDate, Math.round(delta));
  },
);
watch(
  () => form.startTime,
  (v, old) => {
    if (form.allDay || form.startDate !== form.endDate) return;
    if (form.endTime <= v) {
      const [h, m] = v.split(":").map(Number);
      const base = new Date(2000, 0, 1, h, m);
      const prev = old ? old.split(":").map(Number) : [h, m];
      const prevEnd = form.endTime.split(":").map(Number);
      const dur = Math.max(30, (prevEnd[0] * 60 + prevEnd[1]) - (prev[0] * 60 + prev[1]));
      const end = new Date(base.getTime() + dur * 60000);
      if (end.getDate() === 1) form.endTime = timeStr(end);
      else form.endTime = "23:59";
    }
  },
);

function buildDraft(): EventDraft | null {
  if (form.allDay) {
    if (!form.startDate || !form.endDate) return null;
    const end = form.endDate < form.startDate ? form.startDate : form.endDate;
    return {
      title: form.title,
      description: form.description || null,
      location: form.location || null,
      allDay: true,
      start: form.startDate,
      end: addDaysStr(end, 1),
    };
  }
  const start = new Date(`${form.startDate}T${form.startTime || "00:00"}`);
  const end = new Date(`${form.endDate}T${form.endTime || "00:00"}`);
  if (Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) return null;
  if (end <= start) {
    toast("La hora de fin debe ser posterior al inicio", "error");
    return null;
  }
  return {
    title: form.title,
    description: form.description || null,
    location: form.location || null,
    allDay: false,
    start: start.toISOString(),
    end: end.toISOString(),
  };
}

async function save() {
  const draft = buildDraft();
  if (!draft) return;
  if (!editing && !form.calendarId) {
    toast("Elige un calendario", "error");
    return;
  }
  busy.value = true;
  try {
    if (editing) {
      await api.updateEvent(editing.id, draft);
      toast("Evento actualizado", "success", 2500);
    } else {
      await api.createEvent(form.calendarId, draft);
      try {
        localStorage.setItem(LAST_CAL_KEY, form.calendarId);
      } catch {
        /* ignorar */
      }
      toast("Evento creado", "success", 2500);
    }
    emit("changed");
    emit("close");
  } catch (e) {
    toastError("No se pudo guardar", e);
  } finally {
    busy.value = false;
  }
}

async function remove() {
  if (!editing) return;
  if (!confirmDelete.value) {
    confirmDelete.value = true;
    return;
  }
  busy.value = true;
  try {
    await api.deleteEvent(editing.id);
    toast("Evento eliminado", "success", 2500);
    emit("changed");
    emit("close");
  } catch (e) {
    toastError("No se pudo eliminar", e);
  } finally {
    busy.value = false;
  }
}

function openExternal() {
  if (editing?.url) openUrl(editing.url).catch((e) => toastError("No se pudo abrir", e));
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !readOnly) save();
}
</script>

<template>
  <Modal :title="title" @close="emit('close')">
    <form class="event-form" @submit.prevent="save" @keydown="onKey">
      <div class="title-row">
        <span class="dot" :style="{ background: selectedColor }" />
        <input
          v-model="form.title"
          class="input title"
          placeholder="Añade un título"
          :disabled="readOnly"
          autofocus
        />
      </div>

      <div v-if="editing?.recurring" class="note">
        <Icon name="repeat" :size="14" />
        <span v-if="readOnly">Ocurrencia de un evento recurrente (solo lectura aquí).</span>
        <span v-else>Ocurrencia de un evento recurrente: los cambios solo afectan a esta fecha.</span>
      </div>

      <div class="field">
        <label>Calendario</label>
        <select v-if="!editing" v-model="form.calendarId" class="input">
          <option v-for="w in writable" :key="w.calendar.id" :value="w.calendar.id">
            {{ w.calendar.name }} — {{ w.account.email ?? w.account.name }}
          </option>
        </select>
        <div v-else class="static">
          <span class="dot sm" :style="{ background: selectedColor }" />
          {{ owner?.calendar.name ?? "—" }}
          <span class="muted">· {{ owner ? PROVIDERS[owner.account.provider].label : "" }}</span>
          <Icon v-if="readOnly" name="lock" :size="12" />
        </div>
      </div>

      <label class="switch">
        <input v-model="form.allDay" type="checkbox" :disabled="readOnly" />
        Todo el día
      </label>

      <div class="when">
        <div class="field">
          <label>Inicio</label>
          <div class="row">
            <input v-model="form.startDate" type="date" class="input" required :disabled="readOnly" />
            <input
              v-if="!form.allDay"
              v-model="form.startTime"
              type="time"
              class="input"
              required
              :disabled="readOnly"
            />
          </div>
        </div>
        <div class="field">
          <label>Fin</label>
          <div class="row">
            <input
              v-model="form.endDate"
              type="date"
              class="input"
              required
              :min="form.startDate"
              :disabled="readOnly"
            />
            <input
              v-if="!form.allDay"
              v-model="form.endTime"
              type="time"
              class="input"
              required
              :disabled="readOnly"
            />
          </div>
        </div>
      </div>

      <div class="field">
        <label>Ubicación</label>
        <input v-model="form.location" class="input" placeholder="Lugar o enlace de videollamada" :disabled="readOnly" />
      </div>

      <div class="field">
        <label>Descripción</label>
        <textarea v-model="form.description" class="input" rows="4" :disabled="readOnly" />
      </div>
      <button type="submit" hidden />
    </form>

    <template #footer>
      <button v-if="editing && !readOnly" class="btn danger" :disabled="busy" @click="remove">
        <Icon name="trash" /> {{ confirmDelete ? "¿Eliminar de verdad?" : "Eliminar" }}
      </button>
      <button v-if="editing?.url" class="btn ghost" @click="openExternal">
        <Icon name="external" /> Abrir en {{ owner ? PROVIDERS[owner.account.provider].label : "web" }}
      </button>
      <span class="spacer" />
      <button class="btn" @click="emit('close')">{{ readOnly ? "Cerrar" : "Cancelar" }}</button>
      <button v-if="!readOnly" class="btn primary" :disabled="busy" @click="save">
        <span v-if="busy" class="spinner" />
        {{ editing ? "Guardar" : "Crear" }}
      </button>
    </template>
  </Modal>
</template>

<style scoped>
.title-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 14px;
}
.title {
  font-size: 17px;
  font-weight: 600;
}
.dot {
  width: 14px;
  height: 14px;
  border-radius: 4px;
  flex: none;
}
.dot.sm {
  width: 10px;
  height: 10px;
  border-radius: 3px;
}
.static {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 2px;
}
.muted {
  color: var(--text-3);
}
.note {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12.5px;
  color: var(--text-2);
  background: var(--surface-2);
  border-radius: var(--radius-sm);
  padding: 7px 10px;
  margin-bottom: 12px;
}
.switch {
  margin-bottom: 12px;
}
.when {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
</style>
