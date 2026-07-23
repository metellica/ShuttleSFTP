import { defineStore } from 'pinia'
import { ref } from 'vue'
import { listTransfers, clearFinishedTransfers } from '@/composables/useTauri'
import type { TransferTask } from '@/types/transfer'

export const useTransferStore = defineStore('transfer', () => {
  const tasks = ref<TransferTask[]>([])

  function addTask(task: TransferTask) {
    tasks.value.push(task)
  }

  function updateTask(taskId: string, updates: Partial<TransferTask>) {
    const task = tasks.value.find((t) => t.id === taskId)
    if (task) Object.assign(task, updates)
  }

  function removeTask(taskId: string) {
    tasks.value = tasks.value.filter((t) => t.id !== taskId)
  }

  async function clearCompleted() {
    tasks.value = tasks.value.filter(
      (t) => t.status !== 'completed' && t.status !== 'cancelled'
    )
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
  }

  return { tasks, addTask, updateTask, removeTask, clearCompleted, syncTasks }
})
