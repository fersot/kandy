<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type SettingsPayload } from "../api";
import { toast, toastError } from "../store";
import Modal from "./Modal.vue";
import ProviderBadge from "./ProviderBadge.vue";

const emit = defineEmits<{ close: []; saved: [] }>();
const data = ref<SettingsPayload | null>(null);
const busy = ref(false);
const tab = ref<"general" | "google" | "microsoft" | "zoho">("general");

onMounted(async () => {
  try {
    data.value = await api.getSettings();
  } catch (e) {
    toastError("No se pudieron cargar los ajustes", e);
  }
});

const zohoRedirect = computed(
  () => `http://localhost:${data.value?.settings.zohoRedirectPort || 8977}/callback`,
);

function open(url: string) {
  openUrl(url).catch((e) => toastError("No se pudo abrir", e));
}

async function save() {
  if (!data.value) return;
  const s = data.value.settings;
  s.syncIntervalMinutes = Math.min(240, Math.max(5, Number(s.syncIntervalMinutes) || 15));
  s.reminderMinutes = Math.min(1440, Math.max(0, Number(s.reminderMinutes) || 0));
  s.zohoRedirectPort = Math.min(65535, Math.max(1024, Number(s.zohoRedirectPort) || 8977));
  busy.value = true;
  try {
    await api.saveSettings(s);
    toast("Ajustes guardados", "success", 2500);
    emit("saved");
    emit("close");
  } catch (e) {
    toastError("No se pudieron guardar los ajustes", e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <Modal title="Ajustes" wide @close="emit('close')">
    <div v-if="data" class="layout">
      <nav class="tabs">
        <button :class="{ active: tab === 'general' }" @click="tab = 'general'">General</button>
        <div class="tabs-title">Credenciales OAuth</div>
        <button :class="{ active: tab === 'google' }" @click="tab = 'google'">
          <ProviderBadge provider="google" :size="18" /> Google
        </button>
        <button :class="{ active: tab === 'microsoft' }" @click="tab = 'microsoft'">
          <ProviderBadge provider="microsoft" :size="18" /> Microsoft
        </button>
        <button :class="{ active: tab === 'zoho' }" @click="tab = 'zoho'">
          <ProviderBadge provider="zoho" :size="18" /> Zoho
        </button>
      </nav>

      <section v-if="tab === 'general'" class="pane">
        <div class="field">
          <label>Sincronizar automáticamente cada (minutos)</label>
          <input v-model.number="data.settings.syncIntervalMinutes" type="number" min="5" max="240" class="input" />
        </div>
        <div class="field">
          <label>Avisar antes de cada evento (minutos, 0 = nunca)</label>
          <input v-model.number="data.settings.reminderMinutes" type="number" min="0" max="1440" class="input" />
          <p class="hint">Las notificaciones se muestran mientras Kandy está abierto.</p>
        </div>
      </section>

      <section v-else-if="tab === 'google'" class="pane">
        <p v-if="data.builtIn.google" class="ok">Esta compilación ya incluye credenciales de Google. Solo rellena los campos para usar las tuyas.</p>
        <ol class="steps">
          <li>
            En <a href="#" @click.prevent="open('https://console.cloud.google.com/apis/library/calendar-json.googleapis.com')">Google Cloud Console</a>
            habilita <b>Google Calendar API</b>.
          </li>
          <li>Configura la <b>pantalla de consentimiento OAuth</b> (tipo «Externo») y añade tu correo como usuario de prueba.</li>
          <li>
            En <a href="#" @click.prevent="open('https://console.cloud.google.com/apis/credentials')">Credenciales</a> →
            <b>Crear ID de cliente OAuth</b> → tipo <b>App de escritorio</b>. No hace falta URI de redirección.
          </li>
        </ol>
        <div class="field">
          <label>Client ID</label>
          <input v-model="data.settings.googleClientId" class="input mono" placeholder="xxxx.apps.googleusercontent.com" />
        </div>
        <div class="field">
          <label>Client secret</label>
          <input v-model="data.settings.googleClientSecret" class="input mono" placeholder="GOCSPX-…" />
          <p class="hint">Google lo exige también en apps de escritorio; no se considera secreto en este tipo de cliente.</p>
        </div>
      </section>

      <section v-else-if="tab === 'microsoft'" class="pane">
        <p v-if="data.builtIn.microsoft" class="ok">Esta compilación ya incluye credenciales de Microsoft.</p>
        <ol class="steps">
          <li>
            En <a href="#" @click.prevent="open('https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade')">Microsoft Entra → Registros de aplicaciones</a>
            crea un registro nuevo con «Cuentas en cualquier directorio organizativo y cuentas Microsoft personales».
          </li>
          <li>
            En <b>Autenticación</b> añade la plataforma <b>Aplicaciones móviles y de escritorio</b> con la URI
            <span class="code">http://localhost</span> y activa <b>Permitir flujos de clientes públicos</b>.
          </li>
          <li>En <b>Permisos de API</b> añade (delegados) <b>Calendars.ReadWrite</b>, <b>User.Read</b> y <b>offline_access</b>.</li>
        </ol>
        <div class="field">
          <label>Application (client) ID</label>
          <input v-model="data.settings.microsoftClientId" class="input mono" placeholder="00000000-0000-0000-0000-000000000000" />
        </div>
        <div class="field">
          <label>Tenant</label>
          <input v-model="data.settings.microsoftTenant" class="input mono" placeholder="common" />
          <p class="hint"><span class="code">common</span> admite cuentas personales y de empresa. Usa el ID de tu organización para restringirlo.</p>
        </div>
      </section>

      <section v-else-if="tab === 'zoho'" class="pane">
        <p v-if="data.builtIn.zoho" class="ok">Esta compilación ya incluye credenciales de Zoho.</p>
        <ol class="steps">
          <li>
            En <a href="#" @click.prevent="open('https://api-console.zoho.com/')">Zoho API Console</a> → <b>Add Client</b> →
            <b>Mobile-based Applications</b>.
          </li>
          <li>
            URI de redirección autorizada: <span class="code">{{ zohoRedirect }}</span>
          </li>
          <li>En <b>Settings</b> del cliente activa <b>Multi-DC</b> si tienes cuentas en otras regiones.</li>
        </ol>
        <div class="field">
          <label>Client ID</label>
          <input v-model="data.settings.zohoClientId" class="input mono" placeholder="1000.XXXXXXXX" />
        </div>
        <div class="field">
          <label>Client secret</label>
          <input v-model="data.settings.zohoClientSecret" class="input mono" />
        </div>
        <div class="field">
          <label>Puerto local de redirección</label>
          <input v-model.number="data.settings.zohoRedirectPort" type="number" min="1024" max="65535" class="input" />
          <p class="hint">Debe coincidir con la URI registrada en Zoho.</p>
        </div>
      </section>
    </div>

    <template #footer>
      <button class="btn" @click="emit('close')">Cancelar</button>
      <button class="btn primary" :disabled="busy || !data" @click="save">Guardar</button>
    </template>
  </Modal>
</template>

<style scoped>
.layout {
  display: grid;
  grid-template-columns: 170px 1fr;
  gap: 18px;
  min-height: 360px;
}
.tabs {
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-right: 1px solid var(--border);
  padding-right: 12px;
}
.tabs-title {
  font-size: 11px;
  font-weight: 650;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-3);
  margin: 14px 8px 4px;
}
.tabs button {
  display: flex;
  align-items: center;
  gap: 8px;
  text-align: left;
  border: none;
  background: transparent;
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  cursor: pointer;
}
.tabs button:hover {
  background: var(--surface-2);
}
.tabs button.active {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}
.pane {
  min-width: 0;
}
.steps {
  margin: 0 0 16px;
  padding-left: 20px;
  color: var(--text-2);
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.steps a {
  color: var(--accent);
}
.mono {
  font-family: ui-monospace, "JetBrains Mono", monospace;
  font-size: 13px;
}
.ok {
  background: color-mix(in srgb, var(--success) 12%, transparent);
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  margin-top: 0;
}
</style>
