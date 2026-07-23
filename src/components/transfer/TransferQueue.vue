<script setup lang="ts">
import { computed, ref } from 'vue'
import { useTransferStore } from '@/stores/transfer'
import {
  pauseTransfer,
  resumeTransfer,
  cancelTransfer,
  pauseAllTransfers,
  resumeAllTransfers,
  cancelAllTransfers,
  cancelTransferGroup,
} from '@/composables/useTauri'
import { connectForTransferTask } from '@/composables/useAutoConnect'
import { ask } from '@tauri-apps/plugin-dialog'
import type { TransferTask } from '@/types/transfer'

const transferStore = useTransferStore()

/** A directory transfer shown as one expandable tree node. */
interface GroupNode {
  id: string
  name: string
  direction: 'upload' | 'download'
  tasks: TransferTask[]
  status: TransferTask['status']
  transferredBytes: number
  totalBytes: number
  speed: number
  doneCount: number
}
type Row = { type: 'task'; task: TransferTask } | { type: 'group'; group: GroupNode }

const expanded = ref(new Set<string>())

function toggleGroup(id: string) {
  if (expanded.value.has(id)) expanded.value.delete(id)
  else expanded.value.add(id)
}

function deriveGroupStatus(tasks: TransferTask[]): TransferTask['status'] {
  if (tasks.some((t) => t.status === 'active')) return 'active'
  if (tasks.some((t) => t.status === 'queued')) return 'queued'
  if (tasks.some((t) => t.status === 'paused')) return 'paused'
  if (tasks.some((t) => t.status === 'failed')) return 'failed'
  // A partially completed group that was cancelled is still cancelled
  if (tasks.some((t) => t.status === 'cancelled')) return 'cancelled'
  return 'completed'
}

const rows = computed<Row[]>(() => {
  const out: Row[] = []
  const groups = new Map<string, GroupNode>()
  const sorted = [...transferStore.tasks].sort(
    (a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0)
  )
  for (const task of sorted) {
    if (!task.groupId) {
      out.push({ type: 'task', task })
      continue
    }
    let group = groups.get(task.groupId)
    if (!group) {
      group = {
        id: task.groupId,
        name: task.groupName ?? 'folder',
        direction: task.direction,
        tasks: [],
        status: 'queued',
        transferredBytes: 0,
        totalBytes: 0,
        speed: 0,
        doneCount: 0,
      }
      groups.set(task.groupId, group)
      out.push({ type: 'group', group })
    }
    group.tasks.push(task)
  }
  for (const group of groups.values()) {
    group.status = deriveGroupStatus(group.tasks)
    for (const t of group.tasks) {
      group.transferredBytes += t.transferredBytes
      group.totalBytes += t.totalBytes
      if (t.status === 'active') group.speed += t.speed ?? 0
      if (t.status === 'completed') group.doneCount++
    }
  }
  return out
})

const totalSpeed = computed(() =>
  transferStore.tasks
    .filter((t) => t.status === 'active')
    .reduce((sum, t) => sum + (t.speed ?? 0), 0)
)

const hasRunning = computed(() =>
  transferStore.tasks.some((t) => t.status === 'active' || t.status === 'queued')
)
const hasPaused = computed(() => transferStore.tasks.some((t) => t.status === 'paused'))
const hasCancellable = computed(() =>
  transferStore.tasks.some(
    (t) => t.status === 'active' || t.status === 'queued' || t.status === 'paused'
  )
)

async function onPause(task: TransferTask) {
  try {
    await pauseTransfer(task.id)
  } catch (e) {
    console.error('Pause failed:', e)
  }
}

async function onResume(task: TransferTask) {
  try {
    await resumeTransfer(task.id)
  } catch (e) {
    // No live session for this task: auto-connect from saved credentials
    const sessionId = await connectForTransferTask(task)
    if (!sessionId) {
      alert(`Cannot resume: ${e}`)
      return
    }
    try {
      await resumeTransfer(task.id, sessionId)
    } catch (e2) {
      console.error('Resume failed:', e2)
      alert(`Cannot resume: ${e2}`)
    }
  }
}

/** Whether a download task may have a partial local file on disk. */
function hasPartialFile(t: TransferTask): boolean {
  return t.direction === 'download' && (t.status === 'active' || t.status === 'paused')
}

async function confirmDeleteLocal(count: number): Promise<boolean> {
  const what = count === 1 ? 'the partially downloaded file' : `${count} partially downloaded files`
  return await ask(`Delete ${what} from disk?`, {
    title: 'Cancel Download',
    kind: 'warning',
  })
}

async function onCancel(task: TransferTask) {
  try {
    if (task.direction === 'download') {
      const yes = await ask(`Cancel downloading "${taskLabel(task)}"?`, {
        title: 'Cancel Download',
      })
      if (!yes) return
      const deleteLocal = hasPartialFile(task) && (await confirmDeleteLocal(1))
      await cancelTransfer(task.id, deleteLocal)
    } else {
      await cancelTransfer(task.id)
    }
  } catch (e) {
    console.error('Cancel failed:', e)
  }
}

async function onPauseGroup(group: GroupNode) {
  const targets = group.tasks.filter((t) => t.status === 'active' || t.status === 'queued')
  await Promise.allSettled(targets.map((t) => pauseTransfer(t.id)))
}

async function onResumeGroup(group: GroupNode) {
  let sessionId: string | undefined
  let failed = 0
  for (const t of group.tasks.filter(
    (t) => t.status === 'paused' || t.status === 'failed'
  )) {
    try {
      await resumeTransfer(t.id, sessionId)
    } catch {
      // No live session: auto-connect once, then reuse that session
      if (sessionId === undefined) {
        const sid = await connectForTransferTask(t)
        if (sid) {
          sessionId = sid
          try {
            await resumeTransfer(t.id, sid)
            continue
          } catch (e2) {
            console.error('Resume failed:', e2)
          }
        }
      }
      failed++
    }
  }
  if (failed > 0) {
    alert(`${failed} file(s) could not be resumed: connect to the matching server first.`)
  }
}

async function onCancelGroup(group: GroupNode) {
  const targets = group.tasks.filter(
    (t) => t.status === 'active' || t.status === 'queued' || t.status === 'paused'
  )
  if (targets.length === 0) return
  try {
    if (group.direction === 'download') {
      const yes = await ask(`Cancel downloading folder "${group.name}"?`, {
        title: 'Cancel Download',
      })
      if (!yes) return
      const deleteLocal = await ask(
        `Delete the local folder "${group.name}" and everything downloaded into it?`,
        { title: 'Cancel Download', kind: 'warning' }
      )
      await cancelTransferGroup(group.id, deleteLocal)
    } else {
      await Promise.allSettled(targets.map((t) => cancelTransfer(t.id)))
    }
  } catch (e) {
    console.error('Cancel folder failed:', e)
  }
}

function groupCanPause(group: GroupNode): boolean {
  return group.tasks.some((t) => t.status === 'active' || t.status === 'queued')
}

function groupCanResume(group: GroupNode): boolean {
  return group.tasks.some((t) => t.status === 'paused' || t.status === 'failed')
}

function groupCanCancel(group: GroupNode): boolean {
  return group.tasks.some(
    (t) => t.status === 'active' || t.status === 'queued' || t.status === 'paused'
  )
}

async function onPauseAll() {
  try {
    await pauseAllTransfers()
  } catch (e) {
    console.error('Pause all failed:', e)
  }
}

async function onResumeAll() {
  try {
    const resumed = new Set(await resumeAllTransfers())
    // Auto-connect for tasks whose server has no live session
    const remaining = transferStore.tasks.filter(
      (t) => t.status === 'paused' && !resumed.has(t.id)
    )
    if (remaining.length === 0) return

    const groups = new Map<string, TransferTask>()
    for (const t of remaining) {
      if (t.host && !groups.has(`${t.username}@${t.host}`)) {
        groups.set(`${t.username}@${t.host}`, t)
      }
    }
    let connected = false
    for (const t of groups.values()) {
      if (await connectForTransferTask(t)) connected = true
    }
    const retried = new Set(connected ? await resumeAllTransfers() : [])
    const stuck = remaining.filter((t) => !retried.has(t.id))
    if (stuck.length > 0) {
      alert(
        `${stuck.length} transfer(s) could not be resumed automatically. ` +
          'No saved credentials found: connect to the matching server, then resume.'
      )
    }
  } catch (e) {
    console.error('Resume all failed:', e)
  }
}

async function onCancelAll() {
  try {
    const cancellable = transferStore.tasks.filter(
      (t) => t.status === 'active' || t.status === 'queued' || t.status === 'paused'
    )
    if (cancellable.length === 0) return
    const yes = await ask(`Cancel all ${cancellable.length} transfer(s)?`, {
      title: 'Cancel All Transfers',
    })
    if (!yes) return
    const hasDownloads = cancellable.some((t) => t.direction === 'download')
    const deleteLocal =
      hasDownloads &&
      (await ask('Delete partially downloaded files and folders from disk?', {
        title: 'Cancel All Transfers',
        kind: 'warning',
      }))
    await cancelAllTransfers(deleteLocal)
  } catch (e) {
    console.error('Cancel all failed:', e)
  }
}

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec < 1024) return `${Math.round(bytesPerSec)} B/s`
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`
  return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`
}

function progressPercent(task: { transferredBytes: number; totalBytes: number }): number {
  if (task.totalBytes === 0) return 0
  return Math.round((task.transferredBytes / task.totalBytes) * 100)
}

function taskLabel(task: TransferTask): string {
  if (task.relPath) return task.relPath
  return task.sourcePath.split(/[/\\]/).pop() ?? task.sourcePath
}
</script>

<template>
  <div class="transfer-queue" v-if="transferStore.tasks.length > 0">
    <div class="queue-header">
      <span>Transfers ({{ rows.length }})</span>
      <span v-if="totalSpeed > 0" class="total-speed">{{ formatSpeed(totalSpeed) }}</span>
      <span class="header-actions">
        <button v-if="hasRunning" class="hdr-btn" title="Pause all" @click="onPauseAll">⏸ All</button>
        <button v-if="hasPaused" class="hdr-btn" title="Resume all" @click="onResumeAll">▶ All</button>
        <button v-if="hasCancellable" class="hdr-btn danger" title="Cancel all" @click="onCancelAll">✕ All</button>
        <button class="clear-btn" @click="transferStore.clearCompleted">Clear done</button>
      </span>
    </div>
    <div class="queue-list">
      <template v-for="row in rows" :key="row.type === 'task' ? row.task.id : row.group.id">
        <!-- Standalone file task -->
        <div v-if="row.type === 'task'" class="task-row">
          <span class="task-icon">{{ row.task.direction === 'upload' ? '⬆' : '⬇' }}</span>
          <span class="task-name">{{ taskLabel(row.task) }}</span>
          <span class="task-status" :class="row.task.status">{{ row.task.status }}</span>
          <div class="task-progress" v-if="row.task.status === 'active' || row.task.status === 'paused'">
            <span v-if="row.task.status === 'active'" class="task-speed">{{ formatSpeed(row.task.speed ?? 0) }}</span>
            <div class="progress-bar">
              <div class="progress-fill" :class="{ paused: row.task.status === 'paused' }" :style="{ width: progressPercent(row.task) + '%' }" />
            </div>
            <span class="progress-text">{{ progressPercent(row.task) }}%</span>
          </div>
          <span class="task-actions">
            <button
              v-if="row.task.status === 'active' || row.task.status === 'queued'"
              class="act-btn" title="Pause" @click="onPause(row.task)"
            >⏸</button>
            <button
              v-if="row.task.status === 'paused' || row.task.status === 'failed'"
              class="act-btn" :title="row.task.status === 'failed' ? 'Retry' : 'Resume'" @click="onResume(row.task)"
            >▶</button>
            <button
              v-if="row.task.status === 'active' || row.task.status === 'queued' || row.task.status === 'paused'"
              class="act-btn danger" title="Cancel" @click="onCancel(row.task)"
            >✕</button>
          </span>
        </div>

        <!-- Directory transfer group -->
        <template v-else>
          <div class="task-row group-row" @click="toggleGroup(row.group.id)">
            <span class="group-toggle">{{ expanded.has(row.group.id) ? '▾' : '▸' }}</span>
            <span class="task-icon">{{ row.group.direction === 'upload' ? '⬆' : '⬇' }}</span>
            <span class="task-name">📁 {{ row.group.name }}</span>
            <span class="group-count">{{ row.group.doneCount }}/{{ row.group.tasks.length }}</span>
            <span class="task-status" :class="row.group.status">{{ row.group.status }}</span>
            <div class="task-progress" v-if="row.group.status === 'active' || row.group.status === 'paused'">
              <span v-if="row.group.status === 'active'" class="task-speed">{{ formatSpeed(row.group.speed) }}</span>
              <div class="progress-bar">
                <div class="progress-fill" :class="{ paused: row.group.status === 'paused' }" :style="{ width: progressPercent(row.group) + '%' }" />
              </div>
              <span class="progress-text">{{ progressPercent(row.group) }}%</span>
            </div>
            <span class="task-actions" @click.stop>
              <button v-if="groupCanPause(row.group)" class="act-btn" title="Pause folder" @click="onPauseGroup(row.group)">⏸</button>
              <button v-if="groupCanResume(row.group)" class="act-btn" title="Resume folder" @click="onResumeGroup(row.group)">▶</button>
              <button v-if="groupCanCancel(row.group)" class="act-btn danger" title="Cancel folder" @click="onCancelGroup(row.group)">✕</button>
            </span>
          </div>
          <template v-if="expanded.has(row.group.id)">
            <div v-for="task in row.group.tasks" :key="task.id" class="task-row child-row">
              <span class="task-icon">{{ task.direction === 'upload' ? '⬆' : '⬇' }}</span>
              <span class="task-name">{{ taskLabel(task) }}</span>
              <span class="task-status" :class="task.status">{{ task.status }}</span>
              <div class="task-progress" v-if="task.status === 'active' || task.status === 'paused'">
                <span v-if="task.status === 'active'" class="task-speed">{{ formatSpeed(task.speed ?? 0) }}</span>
                <div class="progress-bar">
                  <div class="progress-fill" :class="{ paused: task.status === 'paused' }" :style="{ width: progressPercent(task) + '%' }" />
                </div>
                <span class="progress-text">{{ progressPercent(task) }}%</span>
              </div>
              <span class="task-actions">
                <button
                  v-if="task.status === 'active' || task.status === 'queued'"
                  class="act-btn" title="Pause" @click="onPause(task)"
                >⏸</button>
                <button
                  v-if="task.status === 'paused' || task.status === 'failed'"
                  class="act-btn" :title="task.status === 'failed' ? 'Retry' : 'Resume'" @click="onResume(task)"
                >▶</button>
                <button
                  v-if="task.status === 'active' || task.status === 'queued' || task.status === 'paused'"
                  class="act-btn danger" title="Cancel" @click="onCancel(task)"
                >✕</button>
              </span>
            </div>
          </template>
        </template>
      </template>
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
.task-status.paused { color: #fab387; }
.task-status.completed { color: #a6e3a1; }
.task-status.failed { color: #f38ba8; }
.task-status.cancelled { color: #6c7086; }

.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.hdr-btn {
  background: none;
  border: 1px solid #45475a;
  border-radius: 3px;
  color: #cdd6f4;
  cursor: pointer;
  font-size: 11px;
  padding: 1px 6px;
}

.hdr-btn:hover { background: #313244; }
.hdr-btn.danger { color: #f38ba8; }

.task-actions {
  display: flex;
  gap: 4px;
}

.group-row {
  cursor: pointer;
  user-select: none;
}

.group-row:hover {
  background: #1e1e2e;
}

.group-toggle {
  width: 12px;
  color: #a6adc8;
  font-size: 10px;
}

.group-count {
  font-size: 11px;
  color: #6c7086;
  white-space: nowrap;
}

.child-row {
  padding-left: 40px;
}

.act-btn {
  background: none;
  border: none;
  color: #a6adc8;
  cursor: pointer;
  font-size: 12px;
  padding: 0 2px;
}

.act-btn:hover { color: #cdd6f4; }
.act-btn.danger:hover { color: #f38ba8; }

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

.progress-fill.paused {
  background: #fab387;
}

.progress-text {
  font-size: 11px;
  color: #a6adc8;
  min-width: 32px;
}
</style>
