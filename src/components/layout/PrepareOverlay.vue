<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'
import { usePrepareStore } from '@/stores/prepare'

const prepare = usePrepareStore()

const statusText = computed(() => {
  const n = prepare.done.toLocaleString()
  const t = prepare.total.toLocaleString()
  switch (prepare.phase) {
    case 'scanning':
      return `Scanning… ${n} files found`
    case 'queueing':
      return `Queueing files… ${n} / ${t}`
    case 'deleting':
      return prepare.total > 0 ? `Deleting files… ${n} / ${t}` : 'Deleting…'
  }
  return ''
})

const percent = computed(() => {
  if (prepare.phase === 'scanning' || prepare.total === 0) return null
  return Math.min(100, Math.round((prepare.done / prepare.total) * 100))
})

/** Swallow all keyboard input while blocking; Escape cancels. */
function onKeydown(e: KeyboardEvent) {
  if (!prepare.visible) return
  e.stopPropagation()
  e.preventDefault()
  if (e.key === 'Escape') prepare.cancel()
}

onMounted(() => window.addEventListener('keydown', onKeydown, true))
onUnmounted(() => window.removeEventListener('keydown', onKeydown, true))
</script>

<template>
  <div v-if="prepare.visible" class="prepare-overlay" @click.stop @contextmenu.prevent>
    <div class="prepare-dialog">
      <h3>{{ prepare.label }}</h3>
      <div class="spinner-row">
        <span class="spinner" />
        <span class="status">{{ statusText }}</span>
      </div>
      <div v-if="percent !== null" class="bar">
        <div class="fill" :style="{ width: percent + '%' }" />
      </div>
      <div v-else class="bar indeterminate">
        <div class="fill" />
      </div>
      <div class="actions">
        <button class="cancel-btn" :disabled="prepare.cancelling" @click="prepare.cancel()">
          {{ prepare.cancelling ? 'Cancelling…' : 'Cancel' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.prepare-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 300;
}

.prepare-dialog {
  background: #1e1e2e;
  border: 1px solid #45475a;
  border-radius: 8px;
  padding: 24px;
  width: 380px;
  color: #cdd6f4;
}

h3 {
  font-size: 15px;
  margin-bottom: 14px;
}

.spinner-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}

.spinner {
  width: 16px;
  height: 16px;
  border: 2px solid #45475a;
  border-top-color: #89b4fa;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.status {
  font-size: 13px;
  color: #bac2de;
}

.bar {
  height: 6px;
  background: #313244;
  border-radius: 3px;
  overflow: hidden;
  margin-bottom: 16px;
}

.fill {
  height: 100%;
  background: #89b4fa;
  border-radius: 3px;
  transition: width 0.15s ease;
}

.bar.indeterminate .fill {
  width: 40%;
  animation: slide 1.2s ease-in-out infinite;
}

@keyframes slide {
  0% {
    margin-left: -40%;
  }
  100% {
    margin-left: 100%;
  }
}

.actions {
  display: flex;
  justify-content: flex-end;
}

.cancel-btn {
  padding: 6px 16px;
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
}

.cancel-btn:hover:not(:disabled) {
  background: #45475a;
}

.cancel-btn:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
