<script setup lang="ts">
import { ref, onMounted } from 'vue'
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

onMounted(async () => {
  try {
    sshHosts.value = await loadSshConfig()
  } catch (e) {
    // SSH config may not exist
  }
})

function selectHost(entry: SshHostEntry) {
  host.value = entry.hostname || entry.name
  if (entry.port) port.value = entry.port
  if (entry.user) username.value = entry.user
  if (entry.identityFile) {
    authType.value = 'key'
    keyPath.value = entry.identityFile
  }
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
    emit('connected', sessionId, `${username.value}@${host.value}`)
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

      <!-- SSH Config hosts -->
      <div v-if="sshHosts.length" class="ssh-hosts">
        <label>SSH Config Hosts:</label>
        <div class="host-list">
          <button
            v-for="entry in sshHosts"
            :key="entry.name"
            class="host-btn"
            @click="selectHost(entry)"
          >
            {{ entry.name }}
          </button>
        </div>
      </div>

      <div class="form">
        <div class="field">
          <label>Host</label>
          <input v-model="host" placeholder="hostname or IP" />
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

.ssh-hosts {
  margin-bottom: 16px;
}

.ssh-hosts label {
  font-size: 12px;
  color: #a6adc8;
  display: block;
  margin-bottom: 6px;
}

.host-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.host-btn {
  padding: 4px 10px;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  color: #89b4fa;
  font-size: 12px;
  cursor: pointer;
}

.host-btn:hover {
  background: #45475a;
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
