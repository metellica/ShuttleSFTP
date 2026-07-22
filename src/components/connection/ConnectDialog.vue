<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { loadSshConfig, sshConnect } from '@/composables/useTauri'
import type { SshHostEntry, ConnectParams } from '@/types/connection'

const emit = defineEmits<{
  close: []
  connected: [sessionId: string, label: string]
}>()

const sshHosts = ref<SshHostEntry[]>([])
const host = ref('')
const port = ref(22)
const username = ref('')
const authType = ref<'password' | 'key'>('password')
const password = ref('')
const keyPath = ref('')
const passphrase = ref('')
const connecting = ref(false)
const error = ref('')
const showDropdown = ref(false)
const highlightIndex = ref(0)
const selectedAlias = ref('')

onMounted(async () => {
  try {
    sshHosts.value = await loadSshConfig()
  } catch (e) {
    // SSH config may not exist
  }
})

// Fuzzy search over SSH config hosts
function isSubsequence(query: string, target: string): boolean {
  let i = 0
  for (const ch of target) {
    if (ch === query[i]) i++
    if (i === query.length) return true
  }
  return query.length === 0
}

function matchScore(query: string, entry: SshHostEntry): number {
  const name = entry.name.toLowerCase()
  const hostname = (entry.hostname || '').toLowerCase()
  if (name.startsWith(query) || hostname.startsWith(query)) return 3
  if (name.includes(query) || hostname.includes(query)) return 2
  if (isSubsequence(query, name) || isSubsequence(query, hostname)) return 1
  return 0
}

const filteredHosts = computed(() => {
  const q = host.value.trim().toLowerCase()
  if (!q) return sshHosts.value
  return sshHosts.value
    .map((e) => ({ e, s: matchScore(q, e) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.e)
})

function openDropdown() {
  showDropdown.value = true
  highlightIndex.value = 0
}

function onHostInput() {
  // Manual edit invalidates the previously chosen alias
  selectedAlias.value = ''
  openDropdown()
}

function moveHighlight(delta: number) {
  if (!showDropdown.value || filteredHosts.value.length === 0) return
  const len = filteredHosts.value.length
  highlightIndex.value = (highlightIndex.value + delta + len) % len
}

function chooseHighlighted() {
  if (showDropdown.value) {
    const entry = filteredHosts.value[highlightIndex.value]
    if (entry) selectHost(entry)
  }
}

function selectHost(entry: SshHostEntry) {
  host.value = entry.hostname || entry.name
  selectedAlias.value = entry.name
  if (entry.port) port.value = entry.port
  if (entry.user) username.value = entry.user
  if (entry.identityFile) {
    authType.value = 'key'
    keyPath.value = entry.identityFile
  }
  showDropdown.value = false
}

async function doConnect() {
  error.value = ''
  connecting.value = true

  const params: ConnectParams = {
    host: host.value,
    port: port.value,
    username: username.value,
    auth:
      authType.value === 'password'
        ? { type: 'password', password: password.value }
        : { type: 'key', key_path: keyPath.value, passphrase: passphrase.value || null },
  }

  try {
    const sessionId = await sshConnect(params)
    // Prefer the SSH config alias for the tab label, fall back to host/IP
    emit('connected', sessionId, `${username.value}@${selectedAlias.value || host.value}`)
  } catch (e: any) {
    error.value = e?.toString() || 'Connection failed'
  } finally {
    connecting.value = false
  }
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <h2>Connect to Server</h2>

      <div class="form">
        <div class="field combo">
          <label>Host</label>
          <input
            v-model="host"
            placeholder="hostname or IP — type to search SSH config"
            autocomplete="off"
            @focus="openDropdown"
            @input="onHostInput"
            @blur="showDropdown = false"
            @keydown.down.prevent="moveHighlight(1)"
            @keydown.up.prevent="moveHighlight(-1)"
            @keydown.enter.prevent="chooseHighlighted"
            @keydown.esc="showDropdown = false"
          />
          <div v-if="showDropdown && filteredHosts.length" class="combo-list">
            <div
              v-for="(entry, i) in filteredHosts"
              :key="entry.name"
              class="combo-item"
              :class="{ highlighted: i === highlightIndex }"
              @mousedown.prevent="selectHost(entry)"
              @mousemove="highlightIndex = i"
            >
              <span class="combo-name">{{ entry.name }}</span>
              <span class="combo-host">{{ entry.hostname }}</span>
            </div>
          </div>
        </div>
        <div class="field half">
          <label>Port</label>
          <input v-model.number="port" type="number" />
        </div>
        <div class="field">
          <label>Username</label>
          <input v-model="username" placeholder="root" />
        </div>

        <div class="field">
          <label>Auth Method</label>
          <select v-model="authType">
            <option value="password">Password</option>
            <option value="key">Private Key</option>
          </select>
        </div>

        <div v-if="authType === 'password'" class="field">
          <label>Password</label>
          <input v-model="password" type="password" />
        </div>

        <template v-if="authType === 'key'">
          <div class="field">
            <label>Key Path</label>
            <input v-model="keyPath" placeholder="~/.ssh/id_ed25519" />
          </div>
          <div class="field">
            <label>Passphrase (optional)</label>
            <input v-model="passphrase" type="password" />
          </div>
        </template>
      </div>

      <div v-if="error" class="error">{{ error }}</div>

      <div class="actions">
        <button class="btn cancel" @click="emit('close')">Cancel</button>
        <button class="btn primary" :disabled="connecting" @click="doConnect">
          {{ connecting ? 'Connecting...' : 'Connect' }}
        </button>
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
  width: 440px;
  max-height: 80vh;
  overflow-y: auto;
}

h2 {
  margin-bottom: 16px;
  font-size: 16px;
  color: #cdd6f4;
}

.combo {
  position: relative;
}

.combo-list {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  margin-top: 2px;
  max-height: 220px;
  overflow-y: auto;
  background: #24243a;
  border: 1px solid #45475a;
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  z-index: 20;
}

.combo-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  cursor: pointer;
  font-size: 13px;
}

.combo-item.highlighted {
  background: #45475a;
}

.combo-name {
  color: #89b4fa;
  font-weight: 600;
}

.combo-host {
  color: #a6adc8;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.field label {
  font-size: 12px;
  color: #a6adc8;
}

.field input,
.field select {
  padding: 6px 10px;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  color: #cdd6f4;
  font-size: 13px;
  outline: none;
}

.field input:focus,
.field select:focus {
  border-color: #89b4fa;
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
  gap: 8px;
  margin-top: 20px;
}

.btn {
  padding: 6px 16px;
  border-radius: 4px;
  font-size: 13px;
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

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
