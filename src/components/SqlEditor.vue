<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { EditorState, Compartment } from '@codemirror/state'
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { bracketMatching, syntaxHighlighting, defaultHighlightStyle } from '@codemirror/language'
import { autocompletion, completionKeymap } from '@codemirror/autocomplete'
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search'
import { sql, MySQL } from '@codemirror/lang-sql'
const props = defineProps<{ modelValue: string; schema: Record<string, string[]>; database: string | null; disabled: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [source: string]; run: [mode: 'current' | 'all' | 'selection'] }>()
const host = ref<HTMLElement | null>(null)
let view: EditorView | null = null
const language = new Compartment()
const editable = new Compartment()
const sqlLanguage = () => sql({ dialect: MySQL, schema: props.schema, defaultSchema: props.database ?? undefined, upperCaseKeywords: true })
onMounted(() => {
  view = new EditorView({ parent: host.value!, state: EditorState.create({ doc: props.modelValue, extensions: [
    lineNumbers(), highlightActiveLineGutter(), highlightActiveLine(), drawSelection(), history(), bracketMatching(), highlightSelectionMatches(), syntaxHighlighting(defaultHighlightStyle), autocompletion(), language.of(sqlLanguage()), editable.of(EditorView.editable.of(!props.disabled)),
    keymap.of([{ key: 'Mod-Enter', run: () => { emit('run', 'current'); return true } }, { key: 'Mod-Shift-Enter', run: () => { emit('run', 'all'); return true } }, ...defaultKeymap, ...historyKeymap, ...completionKeymap, ...searchKeymap, indentWithTab]),
    EditorView.updateListener.of(update => { if (update.docChanged) emit('update:modelValue', update.state.doc.toString()) }),
    EditorView.theme({ '&': { height: '100%', backgroundColor: 'var(--editor)', color: 'var(--text)' }, '.cm-scroller': { fontFamily: 'var(--mono)', fontSize: '12px', lineHeight: '1.65' }, '.cm-content': { padding: '14px 0' }, '.cm-gutters': { backgroundColor: 'var(--editor)', color: 'var(--muted)', borderRight: '1px solid var(--border)', minWidth: '43px' }, '.cm-activeLineGutter, .cm-activeLine': { backgroundColor: 'var(--hover)' }, '.cm-cursor': { borderLeftColor: 'var(--accent)' }, '&.cm-focused': { outline: 'none' }, '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: 'var(--selection)' }, '.cm-tooltip': { backgroundColor: 'var(--panel)', border: '1px solid var(--border)', color: 'var(--text)' }, '.cm-tooltip-autocomplete > ul > li[aria-selected]': { backgroundColor: 'var(--accent)', color: 'white' } }),
  ] }) })
})
watch(() => props.modelValue, source => { if (view && source !== view.state.doc.toString()) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } }) })
watch(() => [props.schema, props.database], () => view?.dispatch({ effects: language.reconfigure(sqlLanguage()) }), { deep: true })
watch(() => props.disabled, disabled => view?.dispatch({ effects: editable.reconfigure(EditorView.editable.of(!disabled)) }))
onBeforeUnmount(() => { view?.destroy(); view = null })
defineExpose({ selection: () => view ? view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to) : '', cursor: () => view?.state.selection.main.head ?? 0, focus: () => view?.focus() })
</script>

<template><div ref="host" class="sql-editor" aria-label="SQL 编辑器"></div></template>
