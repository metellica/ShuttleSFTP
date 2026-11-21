<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { listBookmarks } from '@/composables/useTauri'
import { jumpToRecent } from '@/composables/useRemoteJump'
import { useRecentStore } from '@/stores/recent'
import { LOCAL_TAB_LABEL } from '@/types/connection'
import type { Bookmark, RecentDir, ServerRef } from '@/types/connection'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ close: [] }>()

const recent = useRecentStore()
const bookmarks = ref<Bookmark[]>([])
const query = ref('')
const selected = ref(0)
const error = ref('')
const busy = ref(false)
const inputRef = ref<HTMLInputElement | null>(null)
const listRef = ref<HTMLElement | null>(null)

interface Candidate extends ServerRef {
  path: string
  name: string
  pinned: boolean
  score: number
}

const LIMIT = 50

function serverOfBookmark(bm: Bookmark): ServerRef {
  if (bm.kind === 'local' || bm.host === 'local') {
    return { serverKey: 'local', label: LOCAL_TAB_LABEL, kind: 'local', host: 'local', port: 0, username: '' }
  }
  return {
    serverKey: `${bm.username}@${bm.host}:${bm.port}`,
    label: `${bm.username}@${bm.hostAlias || `${bm.host}:${bm.port}`}`,
    kind: 'ssh',
    host: bm.host,
    port: bm.port,
    username: bm.username,
  }
}

/** History plus bookmarks, so a bookmarked folder is always reachable here too. */
const candidates = computed<Candidate[]>(() => {
  const seen = new Set<string>()
  const out: Candidate[] = []
  for (const r of recent.entries) {
    seen.add(`${r.serverKey}\n${r.path}`)
    out.push(r)
  }
  for (const bm of bookmarks.value) {
    const server = serverOfBookmark(bm)
    if (seen.has(`${server.serverKey}\n${bm.path}`)) continue
    out.push({ ...server, path: bm.path, name: bm.alias, pinned: true, score: 0 })
  }
  return out
})

/**
 * `@prod logs` keeps folders on servers whose label contains "prod" and
 * whose path contains "logs". Every plain term must appear in the path
 * or name; folders matching in their own name rank higher.
 */
const results = computed(() => {
  const terms = query.value.toLowerCase().split(/\s+/).filter(Boolean)
  const servers = terms.filter((t) => t.startsWith('@')).map((t) => t.slice(1))
  const words = terms.filter((t) => !t.startsWith('@'))

  const scored: { c: Candidate; rank: number }[] = []
  for (const c of candidates.value) {
    const label = `${c.label} ${c.host}`.toLowerCase()
    if (!servers.every((s) => label.includes(s))) continue
    const haystack = `${c.path} ${c.name}`.toLowerCase()
    if (!words.every((w) => haystack.includes(w))) continue
    const name = c.name.toLowerCase()
    const nameHits = words.filter((w) => name.includes(w)).length
    scored.push({ c, rank: (c.pinned ? 1000 : 0) + (c.score + 0.1) * (1 + nameHits) })
  }
  scored.sort((a, b) => b.rank - a.rank)
  return scored.slice(0, LIMIT).map((s) => s.c)
})

watch(query, () => {
  selected.value = 0
  error.value = ''
})

watch(
  () => props.open,
  async (open) => {
    if (!open) return
    query.value = ''
    selected.value = 0
    error.value = ''
    recent.refresh().catch(() => {})
    listBookmarks()
      .then((b) => (bookmarks.value = b))
      .catch(() => {})
    await nextTick()
    inputRef.value?.focus()
  }
)

function move(delta: number) {
  const n = results.value.length
  if (!n) return
  selected.value = (selected.value + delta + n) % n
  nextTick(() => {
    listRef.value?.children[selected.value]?.scrollIntoView({ block: 'nearest' })
  })
}

async function removeAt(index: number) {
  const hit = results.value[index]
  if (!hit || hit.pinned) return
  try {
    await recent.remove(hit as unknown as RecentDir)
  } catch (e) {
    error.value = String(e)
  }
  const n = results.value.length
  if (selected.value >= n) selected.value = Math.max(0, n - 1)
}

function onDelete(e: KeyboardEvent) {
  // Plain Delete edits text; Ctrl+Delete always removes the selected entry.
  if (query.value && !e.ctrlKey) return
  e.preventDefault()
  removeAt(selected.value)
}

async function accept(index: number) {
  const hit = results.value[index]
  if (!hit || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const failure = await jumpToRecent(hit, hit.path)
    if (failure) error.value = failure
    else emit('close')
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div v-if="open" class="overlay" @mousedown.self="emit('close')">
    <div class="palette" @keydown.esc="emit('close')">
      <input
        ref="inputRef"
        v-model="query"
        spellcheck="false"
        placeholder="Jump to a folder on any server…  (@server to narrow, Enter opens)"
        @keydown.down.prevent="move(1)"
        @keydown.up.prevent="move(-1)"
        @keydown.enter.prevent="accept(selected)"
        @keydown.delete="onDelete"
      />
      <div v-if="error" class="error">{{ error }}</div>
      <ul v-if="results.length" ref="listRef">
        <li
          v-for="(c, i) in results"
          :key="c.serverKey + c.path"
          :class="{ active: i === selected }"
          :title="c.path"
          @mousemove="selected = i"
          @click="accept(i)"
        >
          <span class="name">{{ c.pinned ? '📌 ' : '' }}{{ c.name }}</span>
          <span class="host">{{ c.label }}</span>
          <span class="path">{{ c.path }}</span>
          <button
            v-if="!c.pinned"
            class="del"
            title="Remove from history (Ctrl+Delete)"
            @click.stop="removeAt(i)"
          >×</button>
        </li>
      </ul>
      <div v-else class="empty">No matching folders yet — they appear as you browse</div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding-top: 12vh;
  z-index: 300;
}

.palette {
  width: min(680px, 92vw);
  background: var(--bg-panel);
  border: 1px solid var(--text-disabled);
  border-radius: 8px;
  box-shadow: 0 8px 32px var(--shadow-md);
  overflow: hidden;
}

input {
  width: 100%;
  background: var(--bg-secondary);
  border: none;
  border-bottom: 1px solid var(--border);
  color: var(--text-primary);
  font-size: 13px;
  font-family: inherit;
  padding: 10px 12px;
  outline: none;
}

.error {
  padding: 8px 12px;
  font-size: 12px;
  color: var(--error);
  border-bottom: 1px solid var(--border);
}

ul {
  list-style: none;
  margin: 0;
  padding: 4px;
  max-height: 50vh;
  overflow-y: auto;
}

li {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  grid-template-areas: 'name host' 'path path';
  column-gap: 8px;
  padding: 5px 8px;
  border-radius: 6px;
  cursor: pointer;
}

li.active {
  background: var(--bg-selected);
}

.name {
  grid-area: name;
  font-size: 12px;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.host {
  grid-area: host;
  font-size: 10px;
  color: var(--accent);
  white-space: nowrap;
}

.path {
  grid-area: path;
  font-size: 10px;
  font-family: monospace;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.del {
  position: absolute;
  right: 6px;
  bottom: 3px;
  display: none;
  background: var(--bg-panel);
  border: none;
  border-radius: 4px;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
  padding: 1px 6px;
}

li:hover .del,
li.active .del {
  display: block;
}

.del:hover {
  color: var(--error);
}

.empty {
  padding: 16px;
  font-size: 12px;
  color: var(--text-muted);
  text-align: center;
}
</style>
