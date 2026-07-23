<script setup lang="ts">
import { computed } from 'vue'
import { useTransferStore } from '@/stores/transfer'

const transferStore = useTransferStore()

const totalSpeed = computed(() =>
  transferStore.tasks
    .filter((t) => t.status === 'active')
    .reduce((sum, t) => sum + (t.speed ?? 0), 0)
)

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec < 1024) return `${Math.round(bytesPerSec)} B/s`
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`
  return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`
}

function progressPercent(task: { transferredBytes: number; totalBytes: number }): number {
  if (task.totalBytes === 0) return 0
  return Math.round((task.transferredBytes / task.totalBytes) * 100)
}
</script>

<template>
  <div class="transfer-queue" v-if="transferStore.tasks.length > 0">
    <div class="queue-header">
      <span>Transfers ({{ transferStore.tasks.length }})</span>
      <span v-if="totalSpeed > 0" class="total-speed">{{ formatSpeed(totalSpeed) }}</span>
      <button class="clear-btn" @click="transferStore.clearCompleted">Clear done</button>
    </div>
    <div class="queue-list">
      <div v-for="task in transferStore.tasks" :key="task.id" class="task-row">
        <span class="task-icon">{{ task.direction === 'upload' ? '⬆' : '⬇' }}</span>
        <span class="task-name">{{ task.sourcePath.split('/').pop() }}</span>
        <span class="task-status" :class="task.status">{{ task.status }}</span>
        <div class="task-progress" v-if="task.status === 'active'">
          <span class="task-speed">{{ formatSpeed(task.speed ?? 0) }}</span>
          <div class="progress-bar">
            <div class="progress-fill" :style="{ width: progressPercent(task) + '%' }" />
          </div>
          <span class="progress-text">{{ progressPercent(task) }}%</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.transfer-queue {
  border-top: 1px solid #313244;
  background: #181825;
  max-height: 200px;
  overflow-y: auto;
}

.queue-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 12px;
  font-size: 12px;
  color: #a6adc8;
  border-bottom: 1px solid #313244;
}

.clear-btn {
  background: none;
  border: none;
  color: #89b4fa;
  cursor: pointer;
  font-size: 11px;
}

.task-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 12px;
  font-size: 12px;
}

.task-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.task-status {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 3px;
}

.task-status.queued { color: #f9e2af; }
.task-status.active { color: #89b4fa; }
.task-status.completed { color: #a6e3a1; }
.task-status.failed { color: #f38ba8; }
.task-status.cancelled { color: #6c7086; }

.task-progress {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 240px;
}

.task-speed {
  font-size: 11px;
  color: #94e2d5;
  min-width: 70px;
  text-align: right;
  white-space: nowrap;
}

.total-speed {
  color: #94e2d5;
  font-size: 11px;
  margin-left: auto;
  margin-right: 12px;
}

.progress-bar {
  flex: 1;
  height: 4px;
  background: #313244;
  border-radius: 2px;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: #89b4fa;
  transition: width 0.2s;
}

.progress-text {
  font-size: 11px;
  color: #a6adc8;
  min-width: 32px;
}
</style>
