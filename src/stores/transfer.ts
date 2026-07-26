import { defineStore } from 'pinia'
import { ref } from 'vue'
import { listTransfers, clearFinishedTransfers } from '@/composables/useTauri'
import type { TransferTask } from '@/types/transfer'

export const useTransferStore = defineStore('transfer', () => {
  const tasks = ref<TransferTask[]>([])
  // id -> reactive task proxy, so per-event updates don't linearly
  // scan a potentially huge queue.
  let byId = new Map<string, TransferTask>()

  function reindex() {
    byId = new Map(tasks.value.map((t) => [t.id, t]))
  }

  /** Add a task if unknown (e.g. from a backend `transfer:queued` event). */
  function addTask(task: TransferTask) {
    if (byId.has(task.id)) return
    tasks.value.push(task)
    const stored = tasks.value[tasks.value.length - 1]
    if (stored) byId.set(stored.id, stored)
  }

  function updateTask(taskId: string, updates: Partial<TransferTask>) {
    const task = byId.get(taskId)
    if (task) Object.assign(task, updates)
  }

  /** O(1) lookup by task id. */
  function getTask(taskId: string): TransferTask | undefined {
    return byId.get(taskId)
  }

  function removeTask(taskId: string) {
    if (!byId.delete(taskId)) return
    tasks.value = tasks.value.filter((t) => t.id !== taskId)
  }

  async function clearCompleted() {
    tasks.value = tasks.value.filter(
      (t) => t.status !== 'completed' && t.status !== 'cancelled'
    )
    reindex()
    // Also drop them from the backend so they don't reappear after restart
    try {
      await clearFinishedTransfers()
    } catch (e) {
      console.error('Cannot clear finished transfers:', e)
    }
  }

  /** Pull the authoritative task list from the backend. */
  async function syncTasks() {
    tasks.value = await listTransfers()
    reindex()
  }

  return { tasks, addTask, updateTask, getTask, removeTask, clearCompleted, syncTasks }
})
