<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import {
  loadSshConfig,
  listImportedSshHosts,
  setImportedSshHosts,
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
  JumpHost,
  SessionKind,
} from '@/types/connection'
import { LOCAL_TAB_LABEL } from '@/types/connection'

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
/** Aliases the user imported from ~/.ssh/config; only these are listed. */
const importedNames = ref<Set<string>>(new Set())
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
const jumpHosts = ref<JumpHost[]>([])

onMounted(async () => {
  try {
    sshHosts.value = await loadSshConfig()
  } catch (e) {
    // SSH config may not exist
  }
  try {
    importedNames.value = new Set(await listImportedSshHosts())
  } catch (e) {
    // Imported list may not exist yet
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
  // Only user-imported SSH config hosts: the raw list can be huge.
  const sshOpts: HostOption[] = sshHosts.value
    .filter((e) => importedNames.value.has(e.name))
    .map((e) => ({
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

// --- SSH config import picker ---
const showImport = ref(false)
const importFilter = ref('')
const importChecked = ref<Set<string>>(new Set())

function openImport() {
  importFilter.value = ''
  importChecked.value = new Set(importedNames.value)
  showImport.value = true
}

const importList = computed(() => {
  const q = importFilter.value.trim().toLowerCase()
  if (!q) return sshHosts.value
  return sshHosts.value.filter(
    (e) => e.name.toLowerCase().includes(q) || (e.hostname ?? '').toLowerCase().includes(q)
  )
})

function toggleImport(name: string) {
  const s = new Set(importChecked.value)
  if (s.has(name)) s.delete(name)
  else s.add(name)
  importChecked.value = s
}

/** Check/uncheck every host currently visible through the filter. */
function importSelectAll(select: boolean) {
  const s = new Set(importChecked.value)
  for (const e of importList.value) {
    if (select) s.add(e.name)
    else s.delete(e.name)
  }
  importChecked.value = s
}

async function saveImport() {
  try {
    const names = sshHosts.value
      .map((e) => e.name)
      .filter((n) => importChecked.value.has(n))
    await setImportedSshHosts(names)
    importedNames.value = new Set(names)
    showImport.value = false
  } catch (e: any) {
    error.value = e?.toString() || 'Failed to save imported hosts'
  }
}

function onHostInput() {
  // Manual edit invalidates the previously chosen alias/profile
  selectedAlias.value = ''
  selectedProfileId.value = ''
  fromSshConfig.value = false
  jumpHosts.value = []
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
    jumpHosts.value = p.jumpHosts?.map((jump) => ({ ...jump })) ?? []
    saveNode.value = true
    if (p.authMethod === 'key') {
      authType.value = 'key'
      keyPath.value = p.privateKeyPath || ''
      passphrase.value = p.passphrase || ''
    } else {
      authType.value = 'password'
      password.value = p.password || ''
    }
    savePassword.value = !!(
      p.password ||
      p.passphrase ||
      p.jumpHosts?.some((jump) => jump.password || jump.passphrase)
    )
  } else if (opt.sshEntry) {
    const entry = opt.sshEntry
    host.value = entry.hostname || entry.name
    selectedAlias.value = entry.name
    aliasName.value = entry.name
    selectedProfileId.value = ''
    // SSH config hosts are managed in ~/.ssh/config — don't re-save them
    fromSshConfig.value = true
    jumpHosts.value = entry.jumpHosts.map((jump) => ({ ...jump }))
    saveNode.value = false
    savePassword.value = false
    port.value = entry.port ?? 22
    username.value = entry.user ?? ''
    password.value = ''
    passphrase.value = ''
    if (entry.identityFile) {
      authType.value = 'key'
      keyPath.value = entry.identityFile
    } else {
      authType.value = 'password'
      keyPath.value = ''
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
  if (jumpHosts.value.length) {
    profile.jumpHosts = jumpHosts.value.map((jump) => {
      const saved = { ...jump }
      if (!savePassword.value) {
        delete saved.password
        delete saved.passphrase
      }
      return saved
    })
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
      emit('connected', sessionId, LOCAL_TAB_LABEL, { kind: 'local', params: null })
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
      jumpHosts: jumpHosts.value.map((jump) => ({ ...jump })),
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

      <div class="dialog-body">
      <div v-if="mode === 'ssh'" class="form">
        <div class="row">
          <div class="field combo">
            <div class="label-row">
              <label>Host</label>
              <button
                v-if="sshHosts.length"
                type="button"
                class="import-link"
                :title="`Choose which ~/.ssh/config hosts appear in this list (${importedNames.size} of ${sshHosts.length} imported)`"
                @click="openImport"
              >
                📋 Import ({{ importedNames.size }}/{{ sshHosts.length }})
              </button>
            </div>
            <input
              v-model="host"
              placeholder="hostname or IP — type to search"
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
          <div class="field port">
            <label>Port</label>
            <input v-model.number="port" type="number" />
          </div>
        </div>

        <div class="row">
          <div class="field">
            <label>Username</label>
            <input v-model="username" placeholder="root" />
          </div>
          <div class="field">
            <label>Alias (shown on tab)</label>
            <input v-model="aliasName" placeholder="e.g. prod-server" autocomplete="off" />
          </div>
        </div>

        <div class="row">
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
          <div v-else class="field">
            <label>Key Path</label>
            <input v-model="keyPath" placeholder="~/.ssh/id_ed25519" />
          </div>
        </div>

        <div v-if="authType === 'key'" class="row">
          <div class="field">
            <label>Passphrase (optional)</label>
            <input v-model="passphrase" type="password" />
          </div>
        </div>

        <div v-if="jumpHosts.length" class="jump-list">
          <div class="jump-heading">
            ProxyJump:
            <code>{{ jumpHosts.map((jump) => jump.alias || jump.host).join(' → ') }}</code>
          </div>
          <div v-for="(jump, index) in jumpHosts" :key="index" class="jump-hop">
            <div class="jump-title">
              {{ index + 1 }}. {{ jump.username || username }}@{{ jump.alias || jump.host }}:{{
                jump.port
              }}
            </div>
            <div class="row">
              <div class="field">
                <label>Jump Key Path (optional)</label>
                <input
                  v-model="jump.identityFile"
                  placeholder="Blank to reuse target credentials"
                  autocomplete="off"
                />
              </div>
              <div class="field">
                <label>{{ jump.identityFile ? 'Jump Key Passphrase' : 'Jump Password' }}</label>
                <input
                  v-if="jump.identityFile"
                  v-model="jump.passphrase"
                  type="password"
                  placeholder="Optional"
                />
                <input
                  v-else
                  v-model="jump.password"
                  type="password"
                  placeholder="Blank to reuse target credentials"
                />
              </div>
            </div>
          </div>
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
      </div>

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

    <!-- SSH config host import picker -->
    <div v-if="showImport" class="import-overlay" @click.self="showImport = false">
      <div class="import-panel">
        <h3>Import from ~/.ssh/config</h3>
        <input
          v-model="importFilter"
          class="import-filter"
          placeholder="Filter hosts…"
          autocomplete="off"
        />
        <div class="import-hdr">
          <span>{{ importChecked.size }} selected</span>
          <span>
            <button class="mini" @click="importSelectAll(true)">Select all</button>
            <button class="mini" @click="importSelectAll(false)">Clear</button>
          </span>
        </div>
        <div class="import-list">
          <label v-for="e in importList" :key="e.name" class="import-item">
            <input
              type="checkbox"
              :checked="importChecked.has(e.name)"
              @change="toggleImport(e.name)"
            />
            <span class="combo-name">{{ e.name }}</span>
            <span class="combo-host">{{ e.hostname }}</span>
            <span v-if="e.jumpHosts.length" class="combo-host">
              via {{ e.jumpHosts.map((jump) => jump.alias || jump.host).join(' → ') }}
            </span>
          </label>
          <div v-if="importList.length === 0" class="import-empty">No matching hosts</div>
        </div>
        <div class="import-actions">
          <button class="btn cancel" @click="showImport = false">Cancel</button>
          <button class="btn primary" @click="saveImport">Import</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background: var(--bg-primary);
  border: 1px solid var(--text-disabled);
  border-radius: 8px;
  padding: 20px 24px;
  width: 480px;
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* Scrollable middle section: header and action buttons stay visible. */
.dialog-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  margin: 0 -4px;
  padding: 0 4px;
}

h2 {
  margin-bottom: 12px;
  font-size: 16px;
  color: var(--text-primary);
}

.mode-switch {
  display: flex;
  margin-bottom: 16px;
  border: 1px solid var(--text-disabled);
  border-radius: 6px;
  overflow: hidden;
}

.mode-btn {
  flex: 1;
  padding: 7px 0;
  background: var(--surface);
  border: none;
  color: var(--text-secondary);
  font-size: 13px;
  cursor: pointer;
}

.mode-btn + .mode-btn {
  border-left: 1px solid var(--text-disabled);
}

.mode-btn.active {
  background: var(--scrollbar-thumb);
  color: var(--accent-text);
}

.combo {
  position: relative;
}

.label-row {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
}

.import-link {
  background: none;
  border: none;
  color: var(--accent);
  font-size: 11px;
  cursor: pointer;
  padding: 0;
}

.import-link:hover {
  text-decoration: underline;
}

.import-overlay {
  position: fixed;
  inset: 0;
  background: var(--shadow-sm);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 110;
}

.import-panel {
  background: var(--bg-primary);
  border: 1px solid var(--text-disabled);
  border-radius: 8px;
  padding: 20px;
  width: 420px;
  max-height: 70vh;
  display: flex;
  flex-direction: column;
}

.import-panel h3 {
  font-size: 14px;
  color: var(--text-primary);
  margin-bottom: 10px;
}

.import-filter {
  padding: 6px 10px;
  background: var(--surface);
  border: 1px solid var(--text-disabled);
  border-radius: 4px;
  color: var(--text-primary);
  font-size: 13px;
  margin-bottom: 8px;
}

.import-filter:focus {
  border-color: var(--accent);
  outline: none;
}

.import-hdr {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 11px;
  color: var(--text-secondary);
  margin-bottom: 6px;
}

.mini {
  background: var(--surface);
  border: 1px solid var(--text-disabled);
  color: var(--text-primary);
  border-radius: 4px;
  font-size: 11px;
  padding: 1px 8px;
  cursor: pointer;
  margin-left: 6px;
}

.mini:hover {
  background: var(--text-disabled);
}

.import-list {
  flex: 1;
  min-height: 120px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 4px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.import-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 6px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
}

.import-item:hover {
  background: var(--surface);
}

.import-empty {
  color: var(--text-muted);
  text-align: center;
  padding: 16px 0;
  font-size: 12px;
}

.import-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}

.combo-list {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  margin-top: 2px;
  max-height: 220px;
  overflow-y: auto;
  background: var(--bg-panel);
  border: 1px solid var(--text-disabled);
  border-radius: 6px;
  box-shadow: 0 4px 16px var(--shadow-sm);
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
  background: var(--text-disabled);
}

.combo-name {
  color: var(--accent);
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
  flex-wrap: wrap;
  align-items: center;
  gap: 6px 18px;
  margin-top: 2px;
}

.check {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--text-primary);
  cursor: pointer;
  user-select: none;
}

.check input[type='checkbox'] {
  accent-color: var(--scrollbar-thumb);
  width: 14px;
  height: 14px;
  cursor: pointer;
}

.check.disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.combo-host {
  color: var(--text-secondary);
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
  color: var(--text-secondary);
  font-size: 13px;
  cursor: pointer;
}

.combo-clone:hover {
  border-color: var(--text-disabled);
  background: var(--surface);
  color: var(--accent);
}

.saved {
  margin-top: 12px;
  padding: 8px;
  background: var(--surface);
  border-left: 3px solid var(--success);
  color: var(--success);
  font-size: 12px;
  border-radius: 4px;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.row {
  display: flex;
  gap: 10px;
}

.row .field {
  flex: 1;
  min-width: 0;
}

.row .field.port {
  flex: 0 0 80px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.field label {
  font-size: 12px;
  color: var(--text-secondary);
}

.field input,
.field select {
  padding: 6px 10px;
  background: var(--surface);
  border: 1px solid var(--text-disabled);
  border-radius: 4px;
  color: var(--text-primary);
  font-size: 13px;
  outline: none;
}

.field input:focus,
.field select:focus {
  border-color: var(--accent);
}

.hint {
  margin-top: 8px;
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.5;
}

.hint code {
  color: var(--accent);
  background: var(--surface);
  padding: 0 4px;
  border-radius: 3px;
}

.jump-list {
  padding: 8px;
  border: 1px solid var(--text-disabled);
  border-radius: 4px;
}

.jump-heading,
.jump-title {
  color: var(--text-secondary);
  font-size: 12px;
}

.jump-heading code {
  color: var(--accent);
}

.jump-hop {
  margin-top: 8px;
}

.jump-title {
  margin-bottom: 5px;
}

.error {
  margin-top: 12px;
  padding: 8px;
  background: var(--text-disabled);
  border-left: 3px solid var(--error);
  color: var(--error);
  font-size: 12px;
  border-radius: 4px;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 14px;
  flex-shrink: 0;
}

.btn {
  padding: 6px 16px;
  border-radius: 4px;
  font-size: 13px;
  cursor: pointer;
  border: 1px solid var(--text-disabled);
}

.btn.cancel {
  background: var(--surface);
  color: var(--text-primary);
}

.btn.primary {
  background: var(--accent);
  color: var(--bg-primary);
  border-color: var(--accent);
}

.btn.primary:hover {
  background: var(--accent-hover);
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
