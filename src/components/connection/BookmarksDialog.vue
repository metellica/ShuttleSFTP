<script setup lang="ts">
import { ref, onMounted } from 'vue'
import {
  listBookmarks,
  deleteBookmark,
  sshConnect,
  connectContainer,
  connectPod,
} from '@/composables/useTauri'
import type {
  Bookmark,
  ConnectParams,
  ConnectedMeta,
  ContainerConnectSpec,
  PodConnectSpec,
} from '@/types/connection'

const emit = defineEmits<{
  close: []
  connected: [sessionId: string, label: string, path: string, meta: ConnectedMeta]
}>()

const bookmarks = ref<Bookmark[]>([])
const connectingId = ref<string | null>(null)
const error = ref('')

const KIND_ICONS: Record<string, string> = { ssh: '⌁', container: '▣', pod: '⎈' }

onMounted(async () => {
  try {
    bookmarks.value = await listBookmarks()
  } catch (e) {
    // Bookmarks file may not exist yet
  }
})

function buildParams(bm: Bookmark): ConnectParams | null {
  let auth: ConnectParams['auth']
  if (bm.authMethod === 'key') {
    auth = { type: 'key', key_path: bm.privateKeyPath || '', passphrase: bm.passphrase || null }
  } else if (bm.authMethod === 'agent') {
    auth = { type: 'agent' }
  } else {
    let password = bm.password
    if (!password) {
      const input = prompt(`Password for ${bm.username}@${bm.host}:`)
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
  const kind = bm.kind ?? 'ssh'
  const isLocal = bm.host === 'local'
  const params = isLocal ? null : buildParams(bm)
  if (!isLocal && !params) return
  connectingId.value = bm.id
  try {
    if (kind === 'container' && bm.container) {
      const spec: ContainerConnectSpec = {
        runtime: bm.container.runtime,
        containerId: bm.container.containerId,
        name: bm.container.name,
        via: params ?? undefined,
        preferRootfs: true,
      }
      const sessionId = await connectContainer(spec)
      const name = bm.container.name || bm.container.containerId.slice(0, 12)
      const label = isLocal ? `▣ ${name}` : `▣ ${name} via ${bm.host}`
      emit('connected', sessionId, label, bm.path, {
        kind: 'container',
        params,
        containerSpec: spec,
      })
    } else if (kind === 'pod' && bm.pod) {
      const spec: PodConnectSpec = {
        context: bm.pod.context,
        namespace: bm.pod.namespace,
        pod: bm.pod.pod,
        container: bm.pod.container,
        via: params ?? undefined,
      }
      const sessionId = await connectPod(spec)
      emit('connected', sessionId, `⎈ ${bm.pod.pod}@${bm.pod.namespace}`, bm.path, {
        kind: 'pod',
        params,
        podSpec: spec,
      })
    } else {
      if (!params) return
      const sessionId = await sshConnect(params)
      emit('connected', sessionId, `${bm.username}@${bm.alias}`, bm.path, {
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
  if (bm.kind === 'container' && bm.container) {
    const name = bm.container.name || bm.container.containerId.slice(0, 12)
    return bm.host === 'local' ? name : `${name} @ ${bm.host}`
  }
  if (bm.kind === 'pod' && bm.pod) {
    return `${bm.pod.pod} (${bm.pod.namespace})`
  }
  return `${bm.username}@${bm.host}:${bm.port}`
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
        <div v-for="bm in bookmarks" :key="bm.id" class="item">
          <div class="info" @dblclick="connect(bm)">
            <div class="alias" :title="bm.alias">
              <span class="kind-icon">{{ KIND_ICONS[bm.kind ?? 'ssh'] }}</span>
              {{ bm.alias }}
            </div>
            <div class="detail" :title="`${bookmarkTarget(bm)} ${bm.path}`">
              <span class="remote">{{ bookmarkTarget(bm) }}</span>
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

.item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
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
