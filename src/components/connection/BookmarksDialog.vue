<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import {
  listBookmarks,
  deleteBookmark,
  sshConnect,
  connectLocal,
} from '@/composables/useTauri'
import { promptText } from '@/composables/usePrompt'
import type { Bookmark, ConnectParams, ConnectedMeta } from '@/types/connection'

const emit = defineEmits<{
  close: []
  connected: [sessionId: string, label: string, path: string, meta: ConnectedMeta]
}>()

const bookmarks = ref<Bookmark[]>([])
const connectingId = ref<string | null>(null)
const error = ref('')
/** Expanded server groups; everything starts collapsed. */
const expanded = ref(new Set<string>())

const KIND_ICONS: Record<string, string> = { ssh: '⌁', local: '💻' }

/** Bookmark icon: container/pod paths get their own marker. */
function bookmarkIcon(bm: Bookmark): string {
  if (bm.path.startsWith('/@containers')) return '▣'
  if (bm.path.startsWith('/@pods')) return '⎈'
  return KIND_ICONS[bm.kind ?? 'ssh'] ?? '⌁'
}

/** Bookmarks grouped per remote endpoint (user@host:port / local). */
interface ServerGroup {
  key: string
  label: string
  icon: string
  items: Bookmark[]
}

const groups = computed<ServerGroup[]>(() => {
  const map = new Map<string, ServerGroup>()
  for (const bm of bookmarks.value) {
    const isLocal = bm.kind === 'local' || bm.host === 'local'
    const key = isLocal ? 'local' : `${bm.username}@${bm.host}:${bm.port}`
    let group = map.get(key)
    if (!group) {
      group = {
        key,
        label: bookmarkTarget(bm),
        icon: isLocal ? '💻' : '⌁',
        items: [],
      }
      map.set(key, group)
    }
    group.items.push(bm)
  }
  const out = [...map.values()]
  out.sort((a, b) => a.label.localeCompare(b.label))
  for (const g of out) g.items.sort((a, b) => a.path.localeCompare(b.path))
  return out
})

function toggleGroup(key: string) {
  if (expanded.value.has(key)) expanded.value.delete(key)
  else expanded.value.add(key)
}

onMounted(async () => {
  try {
    bookmarks.value = await listBookmarks()
  } catch (e) {
    // Bookmarks file may not exist yet
  }
})

async function buildParams(bm: Bookmark): Promise<ConnectParams | null> {
  let auth: ConnectParams['auth']
  if (bm.authMethod === 'key') {
    auth = { type: 'key', key_path: bm.privateKeyPath || '', passphrase: bm.passphrase || null }
  } else if (bm.authMethod === 'agent') {
    auth = { type: 'agent' }
  } else {
    let password = bm.password
    if (!password) {
      const input = await promptText(`Password for ${bm.username}@${bm.host}:`, { password: true })
      if (input === null) return null
      password = input
    }
    auth = { type: 'password', password }
  }
  return { host: bm.host, port: bm.port, username: bm.username, auth }
}

async function connect(bm: Bookmark) {
  if (connectingId.value) return
  error.value = ''
  const isLocal = bm.kind === 'local' || bm.host === 'local'
  connectingId.value = bm.id
  try {
    if (isLocal) {
      const sessionId = await connectLocal()
      emit('connected', sessionId, '💻 This Machine', bm.path, {
        kind: 'local',
        params: null,
      })
    } else {
      const params = await buildParams(bm)
      if (!params) return
      const sessionId = await sshConnect(params)
      const label = bm.hostAlias || `${bm.host}:${bm.port}`
      emit('connected', sessionId, `${bm.username}@${label}`, bm.path, {
        kind: 'ssh',
        params,
      })
    }
  } catch (e: any) {
    error.value = e?.toString() || 'Connection failed'
  } finally {
    connectingId.value = null
  }
}

function bookmarkTarget(bm: Bookmark): string {
  if (bm.kind === 'local' || bm.host === 'local') return 'This machine'
  // Prefer the connection alias; fall back to ip:port for old bookmarks
  return `${bm.username}@${bm.hostAlias || `${bm.host}:${bm.port}`}`
}

async function remove(bm: Bookmark) {
  try {
    await deleteBookmark(bm.id)
    bookmarks.value = bookmarks.value.filter((b) => b.id !== bm.id)
  } catch (e: any) {
    error.value = e?.toString() || 'Delete failed'
  }
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <h2>⭐ Bookmarks</h2>

      <div v-if="bookmarks.length === 0" class="empty">
        No bookmarks yet. Right-click a remote folder and choose "Add Bookmark".
      </div>

      <div v-else class="list">
        <template v-for="group in groups" :key="group.key">
          <div class="server-row" @click="toggleGroup(group.key)">
            <span class="server-toggle">{{ expanded.has(group.key) ? '▾' : '▸' }}</span>
            <span class="kind-icon">{{ group.icon }}</span>
            <span class="server-label" :title="group.label">{{ group.label }}</span>
            <span class="server-count">{{ group.items.length }}</span>
          </div>
          <template v-if="expanded.has(group.key)">
            <div v-for="bm in group.items" :key="bm.id" class="item">
              <div class="info" @dblclick="connect(bm)">
                <div class="alias" :title="bm.alias">
                  <span class="kind-icon">{{ bookmarkIcon(bm) }}</span>
                  {{ bm.alias }}
                </div>
                <div class="detail" :title="bm.path">
                  <span class="path">{{ bm.path }}</span>
                </div>
              </div>
              <div class="item-actions">
                <button
                  class="btn primary"
                  :disabled="connectingId !== null"
                  @click="connect(bm)"
                >
                  {{ connectingId === bm.id ? 'Connecting…' : 'Connect' }}
                </button>
                <button
                  class="btn danger"
                  :disabled="connectingId !== null"
                  title="Delete bookmark"
                  @click="remove(bm)"
                >
                  🗑
                </button>
              </div>
            </div>
          </template>
        </template>
      </div>

      <div v-if="error" class="error">{{ error }}</div>

      <div class="actions">
        <button class="btn cancel" @click="emit('close')">Close</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #45475a;
  border-radius: 8px;
  padding: 24px;
  width: 520px;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
}

h2 {
  margin-bottom: 16px;
  font-size: 16px;
  color: #cdd6f4;
}

.empty {
  padding: 24px 0;
  color: #6c7086;
  font-size: 13px;
  text-align: center;
}

.list {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.server-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  background: #2a2a40;
  border: 1px solid #313244;
  border-radius: 6px;
  cursor: pointer;
  user-select: none;
}

.server-row:hover {
  border-color: #45475a;
}

.server-toggle {
  color: #a6adc8;
  font-size: 11px;
  width: 12px;
  flex-shrink: 0;
}

.server-label {
  flex: 1;
  min-width: 0;
  color: #cdd6f4;
  font-weight: 600;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.server-count {
  color: #6c7086;
  font-size: 11px;
  background: #313244;
  border-radius: 8px;
  padding: 1px 8px;
  flex-shrink: 0;
}

.item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 10px;
  margin-left: 22px;
  background: #24243a;
  border: 1px solid #313244;
  border-radius: 6px;
}

.item:hover {
  border-color: #45475a;
}

.info {
  flex: 1;
  min-width: 0;
  cursor: default;
}

.alias {
  color: #89b4fa;
  font-weight: 600;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.kind-icon {
  color: #a6adc8;
  font-size: 11px;
  margin-right: 2px;
}

.detail {
  display: flex;
  gap: 8px;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.remote {
  color: #a6adc8;
  flex-shrink: 0;
}

.path {
  color: #6c7086;
  font-family: monospace;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.error {
  margin-top: 12px;
  padding: 8px;
  background: #45475a;
  border-left: 3px solid #f38ba8;
  color: #f38ba8;
  font-size: 12px;
  border-radius: 4px;
}

.actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 16px;
}

.btn {
  padding: 5px 14px;
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
  border: 1px solid #45475a;
}

.btn.cancel {
  background: #313244;
  color: #cdd6f4;
}

.btn.primary {
  background: #89b4fa;
  color: #1e1e2e;
  border-color: #89b4fa;
}

.btn.primary:hover {
  background: #74c7ec;
}

.btn.danger {
  background: #313244;
  color: #f38ba8;
}

.btn.danger:hover {
  background: #45475a;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
