<script setup lang="ts">
import { computed } from "vue";
import { PROVIDERS, type ProviderKind } from "../api";
import Icon from "./Icon.vue";

const props = defineProps<{ provider: ProviderKind; size?: number }>();
const info = computed(() => PROVIDERS[props.provider]);
const letter = computed(
  () => ({ google: "G", microsoft: "M", zoho: "Z" }) as Record<string, string>,
);
</script>

<template>
  <span
    class="badge"
    :style="{ background: info.color, width: `${size ?? 28}px`, height: `${size ?? 28}px` }"
    :title="info.label"
  >
    <template v-if="letter[provider]">{{ letter[provider] }}</template>
    <Icon v-else :name="provider === 'caldav' ? 'server' : 'rss'" :size="(size ?? 28) * 0.55" />
  </span>
</template>

<style scoped>
.badge {
  display: inline-grid;
  place-items: center;
  border-radius: 8px;
  color: #fff;
  font-weight: 700;
  font-size: 13px;
  flex: none;
}
</style>
