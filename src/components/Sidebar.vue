<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { api, PROVIDERS, type AccountInfo, type CalendarInfo } from "../api";
import { store, toastError, refreshAccounts } from "../store";
import Icon from "./Icon.vue";
import ProviderBadge from "./ProviderBadge.vue";

const emit = defineEmits<{
  newEvent: [];
  addAccount: [];
  settings: [];
  sync: [accountId?: string];
  reconnect: [account: AccountInfo];
  calendarsChanged: [madeVisible?: CalendarInfo];
}>();

const menuFor = ref<string | null>(null);
const confirmRemove = ref<string | null>(null);
const anySyncing = computed(() => store.syncing.size > 0);

function isOauth(a: AccountInfo) {
  return a.provider === "google" || a.provider === "microsoft" || a.provider === "zoho";
}

function closeMenu() {
  menuFor.value = null;
  confirmRemove.value = null;
}
onMounted(() => window.addEventListener("click", closeMenu));
onUnmounted(() => window.removeEventListener("click", closeMenu));

function toggleMenu(id: string) {
  const open = menuFor.value === id;
  closeMenu();
  if (!open) menuFor.value = id;
}

async function toggleVisible(c: CalendarInfo) {
  const visible = !c.visible;
  try {
    await api.setCalendarVisibility(c.id, visible);
    c.visible = visible;
    emit("calendarsChanged", visible ? c : undefined);
  } catch (e) {
    toastError("No se pudo cambiar la visibilidad", e);
  }
}

async function setColor(c: CalendarInfo, ev: Event) {
  const color = (ev.target as HTMLInputElement).value;
  try {
    await api.setCalendarColor(c.id, color);
    c.color = color;
    emit("calendarsChanged");
  } catch (e) {
    toastError("No se pudo cambiar el color", e);
  }
}

async function remove(a: AccountInfo) {
  if (confirmRemove.value !== a.id) {
    confirmRemove.value = a.id;
    return;
  }
  closeMenu();
  try {
    await api.removeAccount(a.id);
    await refreshAccounts();
    emit("calendarsChanged");
  } catch (e) {
    toastError("No se pudo quitar la cuenta", e);
  }
}

function lastSyncText(a: AccountInfo) {
  if (!a.lastSync) return "Sin sincronizar";
  const d = new Date(a.lastSync);
  return `Sincronizado ${d.toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" })}`;
}
</script>

<template>
  <aside class="sidebar">
    <div class="brand">
      <img src="/kandy.svg" alt="" width="30" height="30" />
      <span>Kandy</span>
    </div>

    <button class="btn primary block" @click="emit('newEvent')">
      <Icon name="plus" /> Nuevo evento
    </button>

    <div class="section-title">
      <span>Calendarios</span>
      <button class="icon-btn" title="Añadir cuenta" @click="emit('addAccount')">
        <Icon name="plus" />
      </button>
    </div>

    <div class="accounts">
      <div v-if="!store.accounts.length" class="empty">
        <Icon name="calendar" :size="34" />
        <p>Aún no has conectado ningún calendario.</p>
        <button class="btn" @click="emit('addAccount')">Conectar una cuenta</button>
      </div>

      <section v-for="a in store.accounts" :key="a.id" class="account">
        <div class="account-head">
          <ProviderBadge :provider="a.provider" :size="26" />
          <div class="who" :title="lastSyncText(a)">
            <div class="name">{{ a.name }}</div>
            <div class="email">{{ a.email ?? PROVIDERS[a.provider].label }}</div>
          </div>
          <span v-if="store.syncing.has(a.id)" class="spinner" title="Sincronizando…" />
          <span v-else-if="a.lastError" class="warn" :title="a.lastError">
            <Icon name="alert" :size="15" />
          </span>
          <div class="menu-wrap" @click.stop>
            <button class="icon-btn" title="Opciones" @click="toggleMenu(a.id)">
              <Icon name="more" />
            </button>
            <div v-if="menuFor === a.id" class="menu">
              <button @click="closeMenu(); emit('sync', a.id)"><Icon name="refresh" /> Sincronizar ahora</button>
              <button v-if="isOauth(a)" @click="closeMenu(); emit('reconnect', a)">
                <Icon name="user" /> Volver a conectar
              </button>
              <button class="danger" @click="remove(a)">
                <Icon name="trash" /> {{ confirmRemove === a.id ? "¿Seguro? Pulsa otra vez" : "Quitar cuenta" }}
              </button>
            </div>
          </div>
        </div>

        <div v-if="a.lastError" class="account-error">
          <span>{{ a.lastError }}</span>
          <button v-if="isOauth(a)" class="btn" @click="emit('reconnect', a)">Reconectar</button>
        </div>

        <div v-for="c in a.calendars" :key="c.id" class="cal">
          <button
            class="check"
            :class="{ on: c.visible }"
            :style="{ borderColor: c.color, background: c.visible ? c.color : 'transparent' }"
            :aria-pressed="c.visible"
            :title="c.visible ? 'Ocultar' : 'Mostrar'"
            @click="toggleVisible(c)"
          >
            <Icon v-if="c.visible" name="check" :size="11" />
          </button>
          <span class="cal-name" :title="c.name" @click="toggleVisible(c)">{{ c.name }}</span>
          <Icon v-if="c.readOnly" name="lock" :size="12" class="ro" />
          <label class="swatch" :style="{ background: c.color }" title="Cambiar color">
            <input type="color" :value="c.color" @change="setColor(c, $event)" />
          </label>
        </div>
      </section>
    </div>

    <div class="sidebar-footer">
      <button class="btn ghost" :disabled="anySyncing || !store.accounts.length" @click="emit('sync')">
        <span v-if="anySyncing" class="spinner" />
        <Icon v-else name="refresh" />
        {{ anySyncing ? "Sincronizando…" : "Sincronizar" }}
      </button>
      <button class="btn ghost" @click="emit('settings')"><Icon name="settings" /> Ajustes</button>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: 280px;
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px 14px 10px;
  background: var(--surface);
  border-right: 1px solid var(--border);
  min-height: 0;
}
.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 20px;
  font-weight: 750;
  letter-spacing: -0.02em;
  padding: 0 4px;
}
.section-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  font-weight: 650;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-3);
  padding: 0 4px;
  margin-top: 4px;
}
.accounts {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 14px;
  margin: 0 -6px;
  padding: 0 6px;
}
.empty {
  text-align: center;
  color: var(--text-3);
  padding: 24px 8px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}
.empty p {
  margin: 0 0 4px;
}
.account-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 2px 6px;
}
.who {
  flex: 1;
  min-width: 0;
}
.who .name {
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.who .email {
  font-size: 12px;
  color: var(--text-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.warn {
  color: var(--warning);
  display: inline-flex;
}
.account-error {
  font-size: 12px;
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 8%, transparent);
  border-radius: var(--radius-sm);
  padding: 6px 8px;
  margin: 0 2px 6px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  word-break: break-word;
}
.account-error .btn {
  align-self: flex-start;
  padding: 3px 10px;
  font-size: 12px;
}
.menu-wrap {
  position: relative;
}
.menu {
  position: absolute;
  right: 0;
  top: 32px;
  z-index: 20;
  min-width: 210px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow);
  padding: 4px;
  display: flex;
  flex-direction: column;
}
.menu button {
  display: flex;
  align-items: center;
  gap: 8px;
  text-align: left;
  border: none;
  background: transparent;
  padding: 7px 10px;
  border-radius: 6px;
  cursor: pointer;
}
.menu button:hover {
  background: var(--surface-2);
}
.menu button.danger {
  color: var(--danger);
}
.cal {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 4px 6px 4px 8px;
  border-radius: var(--radius-sm);
}
.cal:hover {
  background: var(--surface-2);
}
.check {
  width: 16px;
  height: 16px;
  border-radius: 5px;
  border: 2px solid;
  display: grid;
  place-items: center;
  padding: 0;
  cursor: pointer;
  color: #fff;
  flex: none;
}
.cal-name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}
.ro {
  color: var(--text-3);
  flex: none;
}
.swatch {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s;
  flex: none;
  position: relative;
  overflow: hidden;
}
.cal:hover .swatch {
  opacity: 1;
}
.swatch input {
  position: absolute;
  inset: 0;
  opacity: 0;
  cursor: pointer;
}
.sidebar-footer {
  display: flex;
  justify-content: space-between;
  border-top: 1px solid var(--border);
  padding-top: 8px;
}
</style>
