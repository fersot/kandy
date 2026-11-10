<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { api, errorText, type AccountInfo, type ProviderKind, type SettingsPayload } from "../api";
import { toast } from "../store";
import Modal from "./Modal.vue";
import Icon from "./Icon.vue";
import ProviderBadge from "./ProviderBadge.vue";

const props = defineProps<{ reconnect?: AccountInfo | null }>();
const emit = defineEmits<{ close: []; added: [account: AccountInfo]; openSettings: [] }>();

type Step = "choose" | ProviderKind;
const step = ref<Step>(props.reconnect ? props.reconnect.provider : "choose");
const busy = ref(false);
const waitingBrowser = ref(false);
const cfg = ref<SettingsPayload | null>(null);

const OPTIONS: { kind: ProviderKind; title: string; desc: string }[] = [
  { kind: "google", title: "Google Calendar", desc: "Gmail y Google Workspace" },
  { kind: "microsoft", title: "Microsoft 365 / Outlook", desc: "Outlook.com, Hotmail, Exchange Online" },
  { kind: "zoho", title: "Zoho Calendar", desc: "Cuentas Zoho de cualquier región" },
  { kind: "caldav", title: "CalDAV", desc: "iCloud, Nextcloud, Fastmail, Yahoo, Radicale…" },
  { kind: "ics", title: "Suscripción ICS", desc: "Festivos, calendarios públicos, enlaces webcal://" },
];

const ZOHO_DCS = [
  ["us", "Estados Unidos (zoho.com)"],
  ["eu", "Europa (zoho.eu)"],
  ["in", "India (zoho.in)"],
  ["au", "Australia (zoho.com.au)"],
  ["jp", "Japón (zoho.jp)"],
  ["uk", "Reino Unido (zoho.uk)"],
  ["ca", "Canadá (zohocloud.ca)"],
  ["sa", "Arabia Saudí (zoho.sa)"],
  ["ae", "Emiratos (zoho.ae)"],
  ["cn", "China (zoho.com.cn)"],
] as const;

const CALDAV_PRESETS = [
  { id: "icloud", label: "iCloud", url: "https://caldav.icloud.com/", hint: "Usa una contraseña específica de app creada en account.apple.com." },
  { id: "fastmail", label: "Fastmail", url: "https://caldav.fastmail.com/", hint: "Crea una contraseña de app con acceso CalDAV en Ajustes → Privacidad y seguridad." },
  { id: "nextcloud", label: "Nextcloud", url: "https://TU-SERVIDOR/remote.php/dav/", hint: "Sustituye TU-SERVIDOR. Recomendado: contraseña de app en Ajustes → Seguridad." },
  { id: "yahoo", label: "Yahoo", url: "https://caldav.calendar.yahoo.com/", hint: "Genera una contraseña de app en la seguridad de tu cuenta Yahoo." },
  { id: "other", label: "Otro servidor", url: "", hint: "URL del servidor o directamente de un calendario CalDAV." },
];

const zohoLocation = ref("us");
const caldav = reactive({ preset: "icloud", name: "", url: CALDAV_PRESETS[0].url, username: "", password: "" });
const ics = reactive({ name: "", url: "", color: "#8b5cf6" });

const preset = computed(() => CALDAV_PRESETS.find((p) => p.id === caldav.preset)!);

onMounted(async () => {
  try {
    cfg.value = await api.getSettings();
  } catch {
    /* sin ajustes: se mostrará el aviso */
  }
});

function configured(kind: ProviderKind): boolean {
  if (!cfg.value) return true;
  const s = cfg.value.settings;
  const b = cfg.value.builtIn;
  if (kind === "google") return b.google || !!s.googleClientId.trim();
  if (kind === "microsoft") return b.microsoft || !!s.microsoftClientId.trim();
  if (kind === "zoho") return b.zoho || !!s.zohoClientId.trim();
  return true;
}

function choosePreset(id: string) {
  caldav.preset = id;
  caldav.url = CALDAV_PRESETS.find((p) => p.id === id)!.url;
}

async function run<T>(fn: () => Promise<T>): Promise<T | null> {
  busy.value = true;
  try {
    return await fn();
  } catch (e) {
    const msg = errorText(e);
    if (msg !== "Operación cancelada") toast(msg, "error");
    return null;
  } finally {
    busy.value = false;
    waitingBrowser.value = false;
  }
}

async function connectOauth(kind: ProviderKind) {
  waitingBrowser.value = true;
  const acc = await run(() =>
    api.connectOauth(kind, kind === "zoho" ? zohoLocation.value : undefined, props.reconnect?.id),
  );
  if (acc) done(acc);
}

async function connectCaldav() {
  const acc = await run(() => api.connectCaldav(caldav.name, caldav.url, caldav.username, caldav.password));
  if (acc) done(acc);
}

async function addIcs() {
  const acc = await run(() => api.addIcs(ics.name, ics.url, ics.color));
  if (acc) done(acc);
}

function done(acc: AccountInfo) {
  toast(`${acc.email ?? acc.name} conectado`, "success");
  emit("added", acc);
  emit("close");
}

function close() {
  if (waitingBrowser.value) api.cancelOauth();
  emit("close");
}

const titleFor = computed(() => {
  if (props.reconnect) return "Volver a conectar la cuenta";
  return step.value === "choose" ? "Añadir calendario" : OPTIONS.find((o) => o.kind === step.value)!.title;
});
</script>

<template>
  <Modal :title="titleFor" @close="close">
    <!-- Paso 1: elegir proveedor -->
    <div v-if="step === 'choose'" class="grid">
      <button v-for="o in OPTIONS" :key="o.kind" class="card" @click="step = o.kind">
        <ProviderBadge :provider="o.kind" :size="34" />
        <div>
          <div class="card-title">{{ o.title }}</div>
          <div class="card-desc">{{ o.desc }}</div>
        </div>
      </button>
    </div>

    <!-- OAuth: Google / Microsoft / Zoho -->
    <div v-else-if="step === 'google' || step === 'microsoft' || step === 'zoho'" class="oauth">
      <div v-if="!configured(step)" class="warnbox">
        <Icon name="alert" />
        <div>
          Falta configurar el <b>Client ID</b> de esta integración. Regístrala una vez en la consola del
          proveedor y pega las credenciales en Ajustes (las instrucciones están allí).
          <div><button class="btn" @click="emit('openSettings')">Abrir ajustes</button></div>
        </div>
      </div>
      <template v-else>
        <p v-if="!waitingBrowser">
          Se abrirá tu navegador para que inicies sesión y autorices a Kandy a leer y modificar tus
          calendarios. Tus credenciales nunca pasan por Kandy: solo se guarda un token en el llavero del
          sistema.
        </p>
        <div v-if="step === 'zoho' && !waitingBrowser" class="field">
          <label>Centro de datos de tu cuenta Zoho</label>
          <select v-model="zohoLocation" class="input">
            <option v-for="[id, label] in ZOHO_DCS" :key="id" :value="id">{{ label }}</option>
          </select>
        </div>
        <div v-if="waitingBrowser" class="waiting">
          <span class="spinner big" />
          <div>
            <b>Esperando la autorización…</b>
            <p>Completa el inicio de sesión en la pestaña que se abrió en tu navegador y vuelve aquí.</p>
          </div>
        </div>
      </template>
    </div>

    <!-- CalDAV -->
    <form v-else-if="step === 'caldav'" @submit.prevent="connectCaldav">
      <div class="presets">
        <button
          v-for="p in CALDAV_PRESETS"
          :key="p.id"
          type="button"
          class="chip"
          :class="{ active: caldav.preset === p.id }"
          @click="choosePreset(p.id)"
        >
          {{ p.label }}
        </button>
      </div>
      <div class="field">
        <label>URL del servidor</label>
        <input v-model="caldav.url" class="input" placeholder="https://servidor/dav/" required />
        <p class="hint">{{ preset.hint }}</p>
      </div>
      <div class="row">
        <div class="field">
          <label>Usuario</label>
          <input v-model="caldav.username" class="input" autocomplete="username" required />
        </div>
        <div class="field">
          <label>Contraseña de aplicación</label>
          <input v-model="caldav.password" class="input" type="password" autocomplete="current-password" required />
        </div>
      </div>
      <div class="field">
        <label>Nombre (opcional)</label>
        <input v-model="caldav.name" class="input" :placeholder="preset.label" />
      </div>
      <button type="submit" hidden />
    </form>

    <!-- ICS -->
    <form v-else-if="step === 'ics'" @submit.prevent="addIcs">
      <div class="field">
        <label>URL del calendario</label>
        <input v-model="ics.url" class="input" placeholder="webcal://… o https://…/calendar.ics" required />
        <p class="hint">Solo lectura. Se actualiza en cada sincronización.</p>
      </div>
      <div class="row">
        <div class="field">
          <label>Nombre (opcional)</label>
          <input v-model="ics.name" class="input" placeholder="Se usa el del calendario" />
        </div>
        <div class="field" style="flex: 0 0 90px">
          <label>Color</label>
          <input v-model="ics.color" type="color" class="input color" />
        </div>
      </div>
      <button type="submit" hidden />
    </form>

    <template v-if="step !== 'choose'" #footer>
      <button v-if="!reconnect" class="btn ghost" :disabled="busy" @click="step = 'choose'">
        <Icon name="back" /> Atrás
      </button>
      <span class="spacer" />
      <button v-if="waitingBrowser" class="btn" @click="api.cancelOauth()">Cancelar</button>
      <template v-else>
        <button class="btn" @click="close">Cancelar</button>
        <button
          v-if="step === 'google' || step === 'microsoft' || step === 'zoho'"
          class="btn primary"
          :disabled="busy || !configured(step)"
          @click="connectOauth(step)"
        >
          Iniciar sesión
        </button>
        <button v-else-if="step === 'caldav'" class="btn primary" :disabled="busy" @click="connectCaldav">
          <span v-if="busy" class="spinner" /> Conectar
        </button>
        <button v-else-if="step === 'ics'" class="btn primary" :disabled="busy" @click="addIcs">
          <span v-if="busy" class="spinner" /> Suscribirse
        </button>
      </template>
    </template>
  </Modal>
</template>

<style scoped>
.grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
  padding-bottom: 8px;
}
.card {
  display: flex;
  gap: 12px;
  align-items: center;
  text-align: left;
  padding: 14px;
  border-radius: var(--radius);
  border: 1px solid var(--border);
  background: var(--surface);
  cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s, transform 0.1s;
}
.card:hover {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.card-title {
  font-weight: 600;
}
.card-desc {
  font-size: 12px;
  color: var(--text-3);
}
.oauth p {
  color: var(--text-2);
  margin-top: 0;
}
.warnbox {
  display: flex;
  gap: 10px;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--warning) 12%, transparent);
  color: var(--text);
}
.warnbox .btn {
  margin-top: 10px;
}
.waiting {
  display: flex;
  gap: 14px;
  align-items: center;
  padding: 16px 4px;
}
.waiting p {
  margin: 4px 0 0;
}
.spinner.big {
  width: 28px;
  height: 28px;
  border-width: 3px;
}
.presets {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 14px;
}
.chip {
  border: 1px solid var(--border);
  background: var(--surface);
  border-radius: 999px;
  padding: 5px 12px;
  cursor: pointer;
}
.chip.active {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
.color {
  padding: 2px;
  height: 36px;
}
</style>
