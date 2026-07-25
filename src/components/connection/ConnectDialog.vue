<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import {
  loadSshConfig,
  sshConnect,
  listProfiles,
  saveProfile,
  connectContainer,
  connectPod,
  listContainers,
  listKubeContexts,
  listKubeNamespaces,
  listKubePods,
} from '@/composables/useTauri'
import { useTabsStore } from '@/stores/tabs'
import type {
  SshHostEntry,
  ConnectParams,
  ConnectionProfile,
  ConnectedMeta,
  ContainerInfo,
  PodInfo,
  SessionKind,
  ContainerConnectSpec,
  PodConnectSpec,
} from '@/types/connection'

const props = defineProps<{
  /** Preselected connection type. */
  initialMode?: SessionKind
  /** Preselected host session for container/pod browsing. */
  viaSessionId?: string
}>()

const emit = defineEmits<{
  close: []
  connected: [sessionId: string, label: string, meta: ConnectedMeta]
}>()

const tabsStore = useTabsStore()

const mode = ref<SessionKind>(props.initialMode ?? 'ssh')

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

// --- Container / pod state ------------------------------------------------

/** Where the engine / kubectl runs: 'local' | '<sessionId>' | 'newhost'. */
const hostChoice = ref<string>(props.viaSessionId ?? 'local')
const containers = ref<ContainerInfo[]>([])
const containersLoading = ref(false)
const containersLoaded = ref(false)
const containerFilter = ref('')
const selectedContainerKey = ref('')
const kubeContexts = ref<string[]>([])
const kubeContext = ref('')
const namespaces = ref<string[]>([])
const namespace = ref('')
const pods = ref<PodInfo[]>([])
const podsLoading = ref(false)
const selectedPodName = ref('')
const podContainer = ref('')

const sshSessionTabs = computed(() =>
  tabsStore.tabs.filter(
    (t) => t.status === 'connected' && t.sessionId && t.kind === 'ssh'
  )
)

/** SSH credential fields are shown for SSH mode and for "New SSH host…". */
const needsSshFields = computed(
  () => mode.value === 'ssh' || hostChoice.value === 'newhost'
)

function buildSshParams(): ConnectParams {
  return {
    host: host.value,
    port: port.value,
    username: username.value,
    auth:
      authType.value === 'password'
        ? { type: 'password', password: password.value }
        : { type: 'key', key_path: keyPath.value, passphrase: passphrase.value || null },
  }
}

/** via args for discovery/connect calls, from the host choice. */
function buildVia(): { viaSessionId?: string; via?: ConnectParams } {
  if (hostChoice.value === 'local') return {}
  if (hostChoice.value === 'newhost') return { via: buildSshParams() }
  return { viaSessionId: hostChoice.value }
}

function hostChoiceLabel(): string {
  if (hostChoice.value === 'local') return 'local'
  if (hostChoice.value === 'newhost') return `${username.value}@${host.value}`
  const tab = tabsStore.tabs.find((t) => t.sessionId === hostChoice.value)
  return tab?.label ?? 'host'
}

const filteredContainers = computed(() => {
  const q = containerFilter.value.trim().toLowerCase()
  if (!q) return containers.value
  return containers.value.filter(
    (c) =>
      c.name.toLowerCase().includes(q) ||
      c.image.toLowerCase().includes(q) ||
      c.id.toLowerCase().startsWith(q) ||
      (c.pod ?? '').toLowerCase().includes(q)
  )
})

const selectedContainer = computed(() =>
  containers.value.find((c) => `${c.runtime}:${c.id}` === selectedContainerKey.value) ?? null
)

const selectedPod = computed(
  () => pods.value.find((p) => p.name === selectedPodName.value) ?? null
)

async function loadContainers() {
  error.value = ''
  containersLoading.value = true
  containers.value = []
  selectedContainerKey.value = ''
  try {
    containers.value = await listContainers(buildVia().viaSessionId, buildVia().via)
    containersLoaded.value = true
  } catch (e: any) {
    error.value = e?.toString() || 'Cannot list containers'
  } finally {
    containersLoading.value = false
  }
}

async function loadKubeMeta() {
  error.value = ''
  const { viaSessionId, via } = buildVia()
  try {
    kubeContexts.value = await listKubeContexts(viaSessionId, via)
  } catch {
    kubeContexts.value = [] // kubectl without contexts still works in-cluster
  }
  try {
    namespaces.value = await listKubeNamespaces(
      kubeContext.value || undefined,
      viaSessionId,
      via
    )
    if (!namespace.value && namespaces.value.includes('default')) {
      namespace.value = 'default'
    }
  } catch (e: any) {
    error.value = e?.toString() || 'Cannot list namespaces'
  }
}

async function loadPods() {
  if (!namespace.value) return
  error.value = ''
  podsLoading.value = true
  pods.value = []
  selectedPodName.value = ''
  podContainer.value = ''
  try {
    const { viaSessionId, via } = buildVia()
    pods.value = await listKubePods(
      namespace.value,
      kubeContext.value || undefined,
      viaSessionId,
      via
    )
  } catch (e: any) {
    error.value = e?.toString() || 'Cannot list pods'
  } finally {
    podsLoading.value = false
  }
}

watch(namespace, () => {
  if (mode.value === 'pod' && namespace.value) loadPods()
})
watch(kubeContext, () => {
  namespaces.value = []
  namespace.value = ''
  pods.value = []
  if (mode.value === 'pod') loadKubeMeta()
})
watch(selectedPodName, () => {
  const p = selectedPod.value
  podContainer.value = p && p.containers.length > 0 ? (p.containers[0] ?? '') : ''
})
watch(hostChoice, () => {
  containersLoaded.value = false
  containers.value = []
  kubeContexts.value = []
  namespaces.value = []
  namespace.value = ''
  pods.value = []
})

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
  if (mode.value === 'container' && hostChoice.value !== 'newhost') {
    loadContainers()
  }
  if (mode.value === 'pod' && hostChoice.value !== 'newhost') {
    loadKubeMeta()
  }
})

function switchMode(m: SessionKind) {
  if (mode.value === m) return
  mode.value = m
  error.value = ''
  if (m === 'container' && !containersLoaded.value && hostChoice.value !== 'newhost') {
    loadContainers()
  }
  if (m === 'pod' && namespaces.value.length === 0 && hostChoice.value !== 'newhost') {
    loadKubeMeta()
  }
}

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

async function persistProfile() {
  const profile: ConnectionProfile = {
    id: selectedProfileId.value || crypto.randomUUID(),
    name: aliasName.value.trim() || selectedAlias.value || host.value,
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
  } catch (e) {
    console.error('Failed to save profile:', e)
  }
}

async function doConnectSsh() {
  const params = buildSshParams()
  const sessionId = await sshConnect(params)
  if (saveNode.value) {
    await persistProfile()
  }
  // Tab label: alias name > ssh config alias > host/IP
  const label = aliasName.value.trim() || selectedAlias.value || host.value
  emit('connected', sessionId, `${username.value}@${label}`, {
    kind: 'ssh',
    params,
  })
}

async function doConnectContainer() {
  const c = selectedContainer.value
  if (!c) {
    error.value = 'Select a container first'
    return
  }
  const { viaSessionId, via } = buildVia()
  const spec: ContainerConnectSpec = {
    runtime: c.runtime,
    containerId: c.id,
    name: c.name || undefined,
    viaSessionId,
    via,
    preferRootfs: true,
  }
  const sessionId = await connectContainer(spec)
  const name = c.name || c.id.slice(0, 12)
  const label =
    hostChoice.value === 'local' ? `▣ ${name}` : `▣ ${name} via ${hostChoiceLabel()}`
  emit('connected', sessionId, label, {
    kind: 'container',
    params: hostChoice.value === 'newhost' ? buildSshParams() : null,
    containerSpec: spec,
  })
}

async function doConnectPod() {
  const p = selectedPod.value
  if (!p) {
    error.value = 'Select a pod first'
    return
  }
  const { viaSessionId, via } = buildVia()
  const spec: PodConnectSpec = {
    context: kubeContext.value || undefined,
    namespace: namespace.value,
    pod: p.name,
    container: podContainer.value || undefined,
    viaSessionId,
    via,
  }
  const sessionId = await connectPod(spec)
  const label = `⎈ ${p.name}@${namespace.value}`
  emit('connected', sessionId, label, {
    kind: 'pod',
    params: hostChoice.value === 'newhost' ? buildSshParams() : null,
    podSpec: spec,
  })
}

async function doConnect() {
  error.value = ''
  connecting.value = true
  try {
    if (mode.value === 'ssh') await doConnectSsh()
    else if (mode.value === 'container') await doConnectContainer()
    else await doConnectPod()
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
        <button
          class="mode-btn"
          :class="{ active: mode === 'ssh' }"
          @click="switchMode('ssh')"
        >
          ⌁ SSH
        </button>
        <button
          class="mode-btn"
          :class="{ active: mode === 'container' }"
          @click="switchMode('container')"
        >
          ▣ Container
        </button>
        <button
          class="mode-btn"
          :class="{ active: mode === 'pod' }"
          @click="switchMode('pod')"
        >
          ⎈ K8s Pod
        </button>
      </div>

      <div class="form">
        <!-- Engine / kubectl location for container & pod modes -->
        <div v-if="mode !== 'ssh'" class="field">
          <label>{{ mode === 'container' ? 'Container Engine On' : 'kubectl Runs On' }}</label>
          <select v-model="hostChoice">
            <option value="local">This machine (local)</option>
            <option
              v-for="t in sshSessionTabs"
              :key="t.sessionId!"
              :value="t.sessionId!"
            >
              {{ t.label }} (connected)
            </option>
            <option value="newhost">New SSH host…</option>
          </select>
        </div>

        <!-- SSH credential fields -->
        <template v-if="needsSshFields">
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
        </template>

        <!-- SSH-only extras -->
        <template v-if="mode === 'ssh'">
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
        </template>

        <!-- Container picker -->
        <template v-if="mode === 'container'">
          <div class="field">
            <div class="picker-head">
              <label>Running Containers</label>
              <button class="btn small" :disabled="containersLoading" @click="loadContainers">
                {{ containersLoading ? 'Loading…' : '🔄 Load' }}
              </button>
            </div>
            <input
              v-model="containerFilter"
              placeholder="filter by name / image / id"
              autocomplete="off"
            />
            <div class="pick-list">
              <div v-if="containersLoading" class="pick-empty">Loading containers…</div>
              <div
                v-else-if="filteredContainers.length === 0"
                class="pick-empty"
              >
                {{ containersLoaded ? 'No running containers found' : 'Click Load to list containers' }}
              </div>
              <div
                v-for="c in filteredContainers"
                :key="c.runtime + ':' + c.id"
                class="pick-item"
                :class="{ selected: selectedContainerKey === c.runtime + ':' + c.id }"
                @click="selectedContainerKey = c.runtime + ':' + c.id"
                @dblclick="selectedContainerKey = c.runtime + ':' + c.id; doConnect()"
              >
                <span class="pick-name">▣ {{ c.name || c.id.slice(0, 12) }}</span>
                <span v-if="c.pod" class="pick-badge">pod: {{ c.pod }}</span>
                <span class="pick-badge">{{ c.runtime }}</span>
                <span class="pick-detail">{{ c.image }}</span>
              </div>
            </div>
          </div>
          <p class="hint">
            Direct rootfs access is used when the host allows it (works for distroless
            images); otherwise falls back to exec + shell inside the container.
          </p>
        </template>

        <!-- Pod picker -->
        <template v-if="mode === 'pod'">
          <div class="field" v-if="kubeContexts.length > 0">
            <label>Context</label>
            <select v-model="kubeContext">
              <option value="">(current context)</option>
              <option v-for="c in kubeContexts" :key="c" :value="c">{{ c }}</option>
            </select>
          </div>
          <div class="field">
            <div class="picker-head">
              <label>Namespace</label>
              <button class="btn small" @click="loadKubeMeta">🔄 Load</button>
            </div>
            <select v-model="namespace">
              <option v-if="namespaces.length === 0" value="" disabled>
                Click Load to list namespaces
              </option>
              <option v-for="ns in namespaces" :key="ns" :value="ns">{{ ns }}</option>
            </select>
          </div>
          <div class="field">
            <label>Pod</label>
            <div class="pick-list">
              <div v-if="podsLoading" class="pick-empty">Loading pods…</div>
              <div v-else-if="pods.length === 0" class="pick-empty">
                {{ namespace ? 'No pods in this namespace' : 'Choose a namespace first' }}
              </div>
              <div
                v-for="p in pods"
                :key="p.name"
                class="pick-item"
                :class="{ selected: selectedPodName === p.name }"
                @click="selectedPodName = p.name"
                @dblclick="selectedPodName = p.name; doConnect()"
              >
                <span class="pick-name">⎈ {{ p.name }}</span>
                <span class="pick-badge">{{ p.phase }}</span>
                <span v-if="p.node" class="pick-detail">on {{ p.node }}</span>
              </div>
            </div>
          </div>
          <div class="field" v-if="selectedPod && selectedPod.containers.length > 1">
            <label>Container</label>
            <select v-model="podContainer">
              <option v-for="c in selectedPod.containers" :key="c" :value="c">{{ c }}</option>
            </select>
          </div>
          <p class="hint">
            Uses kubectl exec (needs pods/exec RBAC and shell tools in the image).
            Distroless images: connect via the node's host session instead.
          </p>
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
  width: 480px;
  max-height: 84vh;
  overflow-y: auto;
}

h2 {
  margin-bottom: 12px;
  font-size: 16px;
  color: #cdd6f4;
}

.mode-switch {
  display: flex;
  gap: 0;
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

.picker-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.pick-list {
  margin-top: 4px;
  max-height: 180px;
  overflow-y: auto;
  border: 1px solid #45475a;
  border-radius: 4px;
  background: #24243a;
}

.pick-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  font-size: 12px;
  cursor: pointer;
  border-bottom: 1px solid #2a2a3d;
}

.pick-item:last-child {
  border-bottom: none;
}

.pick-item:hover {
  background: #313244;
}

.pick-item.selected {
  background: #45475a;
}

.pick-name {
  color: #89b4fa;
  font-weight: 600;
  white-space: nowrap;
}

.pick-badge {
  padding: 1px 6px;
  background: #313244;
  border-radius: 8px;
  font-size: 10px;
  color: #a6adc8;
  white-space: nowrap;
  flex-shrink: 0;
}

.pick-detail {
  color: #6c7086;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pick-empty {
  padding: 14px;
  color: #6c7086;
  font-size: 12px;
  text-align: center;
}

.hint {
  font-size: 11px;
  color: #6c7086;
  line-height: 1.5;
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

.btn.small {
  padding: 2px 8px;
  font-size: 11px;
  background: #313244;
  color: #cdd6f4;
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
