<script setup lang="ts">
withDefaults(defineProps<{ name: string; size?: number }>(), { size: 16 })
const paths: Record<string, string> = {
  database: 'M20 6c0 2.2-3.6 4-8 4S4 8.2 4 6s3.6-4 8-4 8 1.8 8 4ZM4 6v6c0 2.2 3.6 4 8 4s8-1.8 8-4V6M4 12v6c0 2.2 3.6 4 8 4s8-1.8 8-4v-6',
  plug: 'M8 2v5m8-5v5M6 7h12v4a6 6 0 0 1-12 0V7Zm6 10v5',
  table: 'M3 4h18v16H3V4Zm0 5h18M8 9v11m6-11v11',
  code: 'm8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18',
  search: 'M21 21l-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
  settings: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1 1-3Z',
  plus: 'M12 5v14M5 12h14',
  refresh: 'M20 7a9 9 0 1 0 1 8M20 2v6h-6',
  chevron: 'm9 5 7 7-7 7',
  edit: 'm16 3 5 5-12 12-6 1 1-6L16 3Zm-3 3 5 5',
  disconnect: 'm4 4 16 16M8 2v5m8-5v5M6 7h12v4a6 6 0 0 1-12 0V7Zm6 10v5',
  trash: 'M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7',
  view: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Zm10-3a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z',
  folder: 'M3 6h6l2 2h10v12H3V6Z',
}
</script>
<template><svg :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path :d="paths[name] || paths.database" /></svg></template>
