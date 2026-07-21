import { defineStore } from 'pinia'
import { ref } from 'vue'
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

  function clearCompleted() {
    tasks.value = tasks.value.filter(
      (t) => t.status !== 'completed' && t.status !== 'cancelled'
    )
  }

  return { tasks, addTask, updateTask, removeTask, clearCompleted }
})
