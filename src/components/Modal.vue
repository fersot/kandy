<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import Icon from "./Icon.vue";

defineProps<{ title: string; wide?: boolean }>();
const emit = defineEmits<{ close: [] }>();

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="overlay" @mousedown.self="emit('close')">
    <div class="dialog" :class="{ wide }" role="dialog" aria-modal="true" :aria-label="title">
      <header>
        <h2>{{ title }}</h2>
        <button class="icon-btn" aria-label="Cerrar" @click="emit('close')">
          <Icon name="x" />
        </button>
      </header>
      <div class="body"><slot /></div>
      <footer v-if="$slots.footer"><slot name="footer" /></footer>
    </div>
  </div>
</template>
