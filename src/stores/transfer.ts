import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import { listTransfers, clearFinishedTransfers } from '@/composables/useTauri'
import type { TransferTask } from '@/types/transfer'

/**
 * Transfer queue state, tuned for tens of thousands of tasks.
 *
 * Tasks are plain (non-reactive) objects behind a shallowRef; event
 * handlers mutate them at any rate cheaply. Consumers depend on a
 * version counter that is bumped at most every 250ms, so heavy derived
 * state (sorting, grouping) recomputes at a bounded frequency instead
 * of once per backend event.
 */
export const useTransferStore = defineStore('transfer', () => {
  const _tasks = shallowRef<TransferTask[]>([])
  const version = ref(0)
  // id -> task, so per-event updates don't scan the whole queue.
  let byId = new Map<string, TransferTask>()

  let bumpTimer: ReturnType<typeof setTimeout> | null = null
  /** Publish pending mutations to watchers, throttled. */
  function bump() {
    bumpTimer ??= setTimeout(() => {
      bumpTimer = null
      version.value++
    }, 250)
  }

  function reindex() {
    byId = new Map(_tasks.value.map((t) => [t.id, t]))
  }

  /** Throttled view of the task list. */
  const tasks = computed(() => {
    void version.value
    // Fresh array each bump: since Vue 3.4 a computed that re-evaluates
    // to an identical value does not notify dependents, so returning the
    // same (mutated-in-place) array would leave consumers stale.
    return _tasks.value.slice()
  })

  /** Add a task if unknown (e.g. from a backend `transfer:queued` event). */
  function addTask(task: TransferTask) {
    if (byId.has(task.id)) return
    _tasks.value.push(task)
    byId.set(task.id, task)
    bump()
  }

  function updateTask(taskId: string, updates: Partial<TransferTask>) {
    const task = byId.get(taskId)
    if (task) {
      Object.assign(task, updates)
      bump()
    }
  }

  /** O(1) lookup by task id. */
  function getTask(taskId: string): TransferTask | undefined {
    return byId.get(taskId)
  }

  function removeTask(taskId: string) {
    if (!byId.delete(taskId)) return
    _tasks.value = _tasks.value.filter((t) => t.id !== taskId)
    version.value++
  }

  async function clearCompleted() {
    _tasks.value = _tasks.value.filter(
      (t) => t.status !== 'completed' && t.status !== 'cancelled' && t.status !== 'failed'
    )
    reindex()
    version.value++
    // Also drop them from the backend so they don't reappear after restart
    try {
      await clearFinishedTransfers()
    } catch (e) {
      console.error('Cannot clear finished transfers:', e)
    }
  }

  /** Pull the authoritative task list from the backend. */
  async function syncTasks() {
    _tasks.value = await listTransfers()
    reindex()
    version.value++
  }

  return { tasks, version, addTask, updateTask, getTask, removeTask, clearCompleted, syncTasks }
})
