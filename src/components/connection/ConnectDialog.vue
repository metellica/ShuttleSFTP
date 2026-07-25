<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import {
  loadSshConfig,
  sshConnect,
  connectLocal,
  listProfiles,
  saveProfile,
} from '@/composables/useTauri'
import type {
  SshHostEntry,
  ConnectParams,
  ConnectionProfile,
  ConnectedMeta,
  SessionKind,
} from '@/types/connection'

const emit = defineEmits<{
  close: []
  connected: [sessionId: string, label: string, meta: ConnectedMeta]
}>()

const mode = ref<SessionKind>('ssh')

/** Unified dropdown item: saved profile or SSH config host. */
interface HostOption {
  kind: 'profile' | 'ssh'
  name: string
  hostname: string
  profile?: ConnectionProfile
  sshEntry?: SshHostEntry
}

const sshHosts = ref<SshHostEntry[]>([])
const profiles = ref<ConnectionProfile[]>([])
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
const aliasName = ref('')
const saveNode = ref(false)
const savePassword = ref(false)
const selectedProfileId = ref('')
const fromSshConfig = ref(false)
const saveMsg = ref('')

onMounted(async () => {
  try {
    sshHosts.value = await loadSshConfig()
  } catch (e) {
    // SSH config may not exist
  }
  try {
    profiles.value = await listProfiles()
  } catch (e) {
    // Profiles file may not exist
  }
})

const allOptions = computed<HostOption[]>(() => {
  const profileOpts: HostOption[] = profiles.value.map((p) => ({
    kind: 'profile',
    name: p.name,
    hostname: p.host,
    profile: p,
  }))
  const sshOpts: HostOption[] = sshHosts.value.map((e) => ({
    kind: 'ssh',
    name: e.name,
    hostname: e.hostname || '',
    sshEntry: e,
  }))
  return [...profileOpts, ...sshOpts]
})

// Fuzzy search over saved profiles + SSH config hosts
function isSubsequence(query: string, target: string): boolean {
  let i = 0
  for (const ch of target) {
    if (ch === query[i]) i++
    if (i === query.length) return true
  }
  return query.length === 0
}

function matchScore(query: string, opt: HostOption): number {
  const name = opt.name.toLowerCase()
  const hostname = opt.hostname.toLowerCase()
  if (name.startsWith(query) || hostname.startsWith(query)) return 3
  if (name.includes(query) || hostname.includes(query)) return 2
  if (isSubsequence(query, name) || isSubsequence(query, hostname)) return 1
  return 0
}

const filteredOptions = computed(() => {
  const q = host.value.trim().toLowerCase()
  if (!q) return allOptions.value
  return allOptions.value
    .map((o) => ({ o, s: matchScore(q, o) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.o)
})

function openDropdown() {
  showDropdown.value = true
  highlightIndex.value = 0
}

function onHostInput() {
  // Manual edit invalidates the previously chosen alias/profile
  selectedAlias.value = ''
  selectedProfileId.value = ''
  fromSshConfig.value = false
  saveMsg.value = ''
  openDropdown()
}

function moveHighlight(delta: number) {
  if (!showDropdown.value || filteredOptions.value.length === 0) return
  const len = filteredOptions.value.length
  highlightIndex.value = (highlightIndex.value + delta + len) % len
}

function chooseHighlighted() {
  if (showDropdown.value) {
    const opt = filteredOptions.value[highlightIndex.value]
    if (opt) selectOption(opt)
  }
}

function selectOption(opt: HostOption) {
  saveMsg.value = ''
  if (opt.kind === 'profile' && opt.profile) {
    const p = opt.profile
    host.value = p.host
    port.value = p.port
    username.value = p.username
    selectedAlias.value = p.name
    aliasName.value = p.name
    selectedProfileId.value = p.id
    fromSshConfig.value = false
    saveNode.value = true
    if (p.authMethod === 'key') {
      authType.value = 'key'
      keyPath.value = p.privateKeyPath || ''
      passphrase.value = p.passphrase || ''
    } else {
      authType.value = 'password'
      password.value = p.password || ''
    }
    savePassword.value = !!(p.password || p.passphrase)
  } else if (opt.sshEntry) {
    const entry = opt.sshEntry
    host.value = entry.hostname || entry.name
    selectedAlias.value = entry.name
    aliasName.value = entry.name
    selectedProfileId.value = ''
    // SSH config hosts are managed in ~/.ssh/config — don't re-save them
    fromSshConfig.value = true
    saveNode.value = false
    savePassword.value = false
    if (entry.port) port.value = entry.port
    if (entry.user) username.value = entry.user
    if (entry.identityFile) {
      authType.value = 'key'
      keyPath.value = entry.identityFile
    }
  }
  showDropdown.value = false
}

function allAliasNames(): Set<string> {
  const names = new Set(profiles.value.map((p) => p.name))
  for (const e of sshHosts.value) names.add(e.name)
  return names
}

function uniqueName(base: string): string {
  const names = allAliasNames()
  if (!names.has(base)) return base
  let i = 2
  while (names.has(`${base} ${i}`)) i++
  return `${base} ${i}`
}

/** Alias taken by another saved profile or any SSH config host. */
function aliasConflicts(name: string): boolean {
  if (profiles.value.some((p) => p.name === name && p.id !== selectedProfileId.value))
    return true
  return sshHosts.value.some((e) => e.name === name)
}

/** Fork the current form into a new profile: same values, new identity. */
function cloneAsNew() {
  const base = (aliasName.value.trim() || selectedAlias.value || host.value) + ' copy'
  aliasName.value = uniqueName(base)
  selectedProfileId.value = ''
  selectedAlias.value = ''
  fromSshConfig.value = false
  saveNode.value = true
  saveMsg.value = ''
}

/** Fill the form from a saved profile, then detach it as a new copy. */
function cloneOption(opt: HostOption) {
  selectOption(opt)
  cloneAsNew()
}

async function persistProfile() {
  const name = aliasName.value.trim() || selectedAlias.value || host.value
  // Alias must be globally unique across saved profiles and SSH config hosts
  if (aliasConflicts(name)) {
    throw new Error(`Alias "${name}" already exists — choose a different name`)
  }
  const profile: ConnectionProfile = {
    id: selectedProfileId.value || crypto.randomUUID(),
    name,
    host: host.value,
    port: port.value,
    username: username.value,
    authMethod: authType.value,
  }
  if (authType.value === 'key') {
    profile.privateKeyPath = keyPath.value
    if (savePassword.value && passphrase.value) profile.passphrase = passphrase.value
  } else if (savePassword.value && password.value) {
    profile.password = password.value
  }
  try {
    await saveProfile(profile)
    selectedProfileId.value = profile.id
    selectedAlias.value = profile.name
    aliasName.value = profile.name
    try {
      profiles.value = await listProfiles()
    } catch (e) {
      // ignore refresh failure
    }
  } catch (e) {
    console.error('Failed to save profile:', e)
    throw e
  }
}

/** Save/update the profile without connecting. */
async function saveOnly() {
  error.value = ''
  saveMsg.value = ''
  if (!host.value.trim()) {
    error.value = 'Host is required'
    return
  }
  try {
    await persistProfile()
    saveNode.value = true
    saveMsg.value = `Saved "${aliasName.value}"`
  } catch (e: any) {
    error.value = e?.toString() || 'Failed to save'
  }
}

async function doConnect() {
  error.value = ''
  connecting.value = true
  try {
    if (mode.value === 'local') {
      const sessionId = await connectLocal()
      emit('connected', sessionId, '💻 This Machine', { kind: 'local', params: null })
      return
    }
    if (saveNode.value && !fromSshConfig.value) {
      // Fail fast on duplicate alias before opening the connection
      const name = aliasName.value.trim() || selectedAlias.value || host.value
      if (aliasConflicts(name)) {
        error.value = `Alias "${name}" already exists — choose a different name`
        return
      }
    }
    const params: ConnectParams = {
      host: host.value,
      port: port.value,
      username: username.value,
      auth:
        authType.value === 'password'
          ? { type: 'password', password: password.value }
          : { type: 'key', key_path: keyPath.value, passphrase: passphrase.value || null },
    }
    const sessionId = await sshConnect(params)
    if (saveNode.value) {
      // Don't let a save failure block a successful connection
      await persistProfile().catch(() => {})
    }
    // Tab label: alias name > ssh config alias > host/IP
    const label = aliasName.value.trim() || selectedAlias.value || host.value
    emit('connected', sessionId, `${username.value}@${label}`, { kind: 'ssh', params })
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
      <h2>New Connection</h2>

      <div class="mode-switch">
        <button class="mode-btn" :class="{ active: mode === 'ssh' }" @click="mode = 'ssh'">
          ⌁ SSH Host
        </button>
        <button class="mode-btn" :class="{ active: mode === 'local' }" @click="mode = 'local'">
          💻 This Machine
        </button>
      </div>

      <div v-if="mode === 'ssh'" class="form">
        <div class="field combo">
          <label>Host</label>
          <input
            v-model="host"
            placeholder="hostname or IP — type to search saved & SSH config hosts"
            autocomplete="off"
            @focus="openDropdown"
            @input="onHostInput"
            @blur="showDropdown = false"
            @keydown.down.prevent="moveHighlight(1)"
            @keydown.up.prevent="moveHighlight(-1)"
            @keydown.enter.prevent="chooseHighlighted"
            @keydown.esc="showDropdown = false"
          />
          <div v-if="showDropdown && filteredOptions.length" class="combo-list">
            <div
              v-for="(opt, i) in filteredOptions"
              :key="opt.kind + ':' + opt.name"
              class="combo-item"
              :class="{ highlighted: i === highlightIndex }"
              @mousedown.prevent="selectOption(opt)"
              @mousemove="highlightIndex = i"
            >
              <span class="combo-kind">{{ opt.kind === 'profile' ? '⭐' : '📋' }}</span>
              <span class="combo-name">{{ opt.name }}</span>
              <span class="combo-host">{{ opt.hostname }}</span>
              <button
                class="combo-clone"
                title="Clone this connection"
                @mousedown.prevent.stop="cloneOption(opt)"
              >
                ⧉
              </button>
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

        <div class="field">
          <label>Alias Name (shown on tab)</label>
          <input v-model="aliasName" placeholder="e.g. prod-server" autocomplete="off" />
        </div>

        <div class="checks">
          <label class="check" :class="{ disabled: fromSshConfig }">
            <input type="checkbox" v-model="saveNode" :disabled="fromSshConfig" />
            <span>Save connection{{ fromSshConfig ? ' (managed by ~/.ssh/config)' : '' }}</span>
          </label>
          <label class="check" :class="{ disabled: !saveNode || fromSshConfig }">
            <input
              type="checkbox"
              v-model="savePassword"
              :disabled="!saveNode || fromSshConfig"
            />
            <span>Save {{ authType === 'key' ? 'passphrase' : 'password' }} (plain text)</span>
          </label>
        </div>
      </div>

      <div v-else class="form">
        <p class="hint">
          Browse this machine's files, plus its running containers under
          <code>/@containers</code> and K8s pods under <code>/@pods</code>
          (Docker Desktop, nerdctl, kubectl…).
        </p>
      </div>

      <p v-if="mode === 'ssh'" class="hint">
        After connecting, the host's containers and pods appear as
        <code>/@containers</code> and <code>/@pods</code> in the file browser.
      </p>

      <div v-if="error" class="error">{{ error }}</div>
      <div v-if="saveMsg" class="saved">{{ saveMsg }}</div>

      <div class="actions">
        <button class="btn cancel" @click="emit('close')">Cancel</button>
        <button
          v-if="mode === 'ssh' && !fromSshConfig"
          class="btn"
          :title="selectedProfileId ? 'Update the saved connection without connecting' : 'Save without connecting'"
          @click="saveOnly"
        >
          {{ selectedProfileId ? 'Save Changes' : 'Save' }}
        </button>
        <button class="btn primary" :disabled="connecting" @click="doConnect">
          {{ connecting ? 'Connecting...' : mode === 'local' ? 'Open' : 'Connect' }}
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
  margin-bottom: 12px;
  font-size: 16px;
  color: #cdd6f4;
}

.mode-switch {
  display: flex;
  margin-bottom: 16px;
  border: 1px solid #45475a;
  border-radius: 6px;
  overflow: hidden;
}

.mode-btn {
  flex: 1;
  padding: 7px 0;
  background: #313244;
  border: none;
  color: #a6adc8;
  font-size: 13px;
  cursor: pointer;
}

.mode-btn + .mode-btn {
  border-left: 1px solid #45475a;
}

.mode-btn.active {
  background: #4f6ec2;
  color: #fff;
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
  display: grid;
  grid-template-columns: auto 1fr auto auto;
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
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.combo-kind {
  font-size: 11px;
  flex-shrink: 0;
}

.checks {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 2px;
}

.check {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #cdd6f4;
  cursor: pointer;
  user-select: none;
}

.check input[type='checkbox'] {
  accent-color: #4f6ec2;
  width: 14px;
  height: 14px;
  cursor: pointer;
}

.check.disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.combo-host {
  color: #a6adc8;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.combo-clone {
  flex-shrink: 0;
  padding: 0 6px;
  background: transparent;
  border: 1px solid transparent;
  border-radius: 4px;
  color: #a6adc8;
  font-size: 13px;
  cursor: pointer;
}

.combo-clone:hover {
  border-color: #45475a;
  background: #313244;
  color: #89b4fa;
}

.saved {
  margin-top: 12px;
  padding: 8px;
  background: #313244;
  border-left: 3px solid #a6e3a1;
  color: #a6e3a1;
  font-size: 12px;
  border-radius: 4px;
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

.hint {
  margin-top: 10px;
  font-size: 11px;
  color: #6c7086;
  line-height: 1.6;
}

.hint code {
  color: #89b4fa;
  background: #313244;
  padding: 0 4px;
  border-radius: 3px;
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
