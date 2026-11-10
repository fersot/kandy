<script setup lang="ts">
import { store, dismiss } from "../store";
import Icon from "./Icon.vue";
</script>

<template>
  <div class="toasts" aria-live="polite">
    <div v-for="t in store.toasts" :key="t.id" class="toast" :class="t.kind">
      <span>{{ t.text }}</span>
      <button class="icon-btn" aria-label="Cerrar aviso" @click="dismiss(t.id)">
        <Icon name="x" :size="14" />
      </button>
    </div>
  </div>
</template>
