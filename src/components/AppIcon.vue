<script setup lang="ts">
import { computed } from "vue";

type IconDef = { paths: string[]; filled?: boolean };

const ICONS: Record<string, IconDef> = {
  plus: { paths: ["M12 5v14", "M5 12h14"] },
  settings: {
    paths: ["M4 21v-7", "M4 10V3", "M12 21v-9", "M12 8V3", "M20 21v-5", "M20 12V3", "M1 14h6", "M9 8h6", "M17 16h6"],
  },
  play: { paths: ["M7 4.5l12 7.5-12 7.5z"], filled: true },
  edit: {
    paths: [
      "M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7",
      "M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z",
    ],
  },
  trash: {
    paths: [
      "M3 6h18",
      "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2",
      "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6",
      "M10 11v6",
      "M14 11v6",
    ],
  },
  back: { paths: ["M19 12H5", "M12 19l-7-7 7-7"] },
  refresh: {
    paths: ["M23 4v6h-6", "M1 20v-6h6", "M20.49 9A9 9 0 0 0 5.64 5.64L1 10", "M3.51 15a9 9 0 0 0 14.85 3.36L23 14"],
  },
  external: {
    paths: [
      "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6",
      "M15 3h6v6",
      "M10 14L21 3",
    ],
  },
  terminal: { paths: ["M4 17l6-6-6-6", "M12 19h8"] },
  star: {
    paths: [
      "M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z",
    ],
  },
  'star-fill': {
    paths: ["M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z"],
    filled: true,
  },
  check: { paths: ["M20 6L9 17l-5-5"] },
  close: { paths: ["M18 6L6 18", "M6 6l12 12"] },
  minimize: { paths: ["M5 12h14"] },
  maximize: { paths: ["M5 5h14v14H5z"] },
  restore: {
    paths: ["M8 8h11v11H8z", "M5 16V5h11"],
  },
  list: {
    paths: ["M8 6h13", "M8 12h13", "M8 18h13", "M3.5 6h.01", "M3.5 12h.01", "M3.5 18h.01"],
  },
  alert: {
    paths: ["M12 9v4", "M12 17h.01", "M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"],
  },
  link: {
    paths: [
      "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71",
      "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71",
    ],
  },
  stop: { paths: ["M6 6h12v12H6z"], filled: true },
  eye: {
    paths: [
      "M1 12s4-7.5 11-7.5S23 12 23 12s-4 7.5-11 7.5S1 12 1 12z",
      "M12 15.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4z",
    ],
  },
  rocket: {
    paths: [
      "M12 2c3.5 2.2 5.5 6 5.5 10.5L12 16l-5.5-3.5C6.5 8 8.5 4.2 12 2z",
      "M12 9.5v.01",
      "M6.5 12.5L4 17l3.5-.8",
      "M17.5 12.5L20 17l-3.5-.8",
      "M9.5 20.5c1.4.7 3.6.7 5 0",
    ],
  },
};

const props = withDefaults(
  defineProps<{ name: keyof typeof ICONS | string; size?: number }>(),
  { size: 16 },
);

const def = computed<IconDef>(() => ICONS[props.name] ?? { paths: [] });
</script>

<template>
  <svg :width="props.size" :height="props.size" viewBox="0 0 24 24" :fill="def.filled ? 'currentColor' : 'none'"
    :stroke="def.filled ? 'none' : 'currentColor'" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"
    aria-hidden="true">
    <path v-for="(d, index) in def.paths" :key="index" :d="d" />
  </svg>
</template>
