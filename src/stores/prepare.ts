import { ref } from 'vue'
import { defineStore } from 'pinia'
import { cancelPrepare } from '@/composables/useTauri'

export type PreparePhase = 'scanning' | 'queueing' | 'deleting' | 'downloading'

export interface PrepareProgressEvent {
  prepareId: string
  phase: PreparePhase
  done: number
  total: number
}

/** Marker for silently swallowed user-cancelled preparations. */
export const PREPARE_CANCELLED = 'Preparation cancelled'

/**
 * Blocking "preparing" overlay state for long bulk operations
 * (directory transfer queueing, recursive deletes). Only one runs at a
 * time; the overlay blocks all interaction until the backend command
 * finishes or the user cancels.
 */
export const usePrepareStore = defineStore('prepare', () => {
  const visible = ref(false)
  const label = ref('')
  const phase = ref<PreparePhase>('scanning')
  const done = ref(0)
  const total = ref(0)
  const cancelling = ref(false)

  let currentId: string | null = null
  let showTimer: ReturnType<typeof setTimeout> | null = null

  /** Feed of backend `prepare:progress` events (wired up in App.vue). */
  function onProgress(p: PrepareProgressEvent) {
    if (p.prepareId !== currentId) return
    phase.value = p.phase
    done.value = p.done
    total.value = p.total
  }

  /**
   * Run `fn` with a fresh prepareId and the blocking overlay. The
   * overlay only appears after a short delay so quick operations don't
   * flash. Returns undefined when the user cancelled.
   */
  async function run<T>(
    labelText: string,
    fn: (prepareId: string) => Promise<T>
  ): Promise<T | undefined> {
    const id = crypto.randomUUID()
    currentId = id
    label.value = labelText
    phase.value = 'scanning'
    done.value = 0
    total.value = 0
    cancelling.value = false
    showTimer = setTimeout(() => {
      visible.value = true
    }, 200)
    try {
      return await fn(id)
    } catch (e) {
      if (String(e).includes(PREPARE_CANCELLED)) return undefined
      throw e
    } finally {
      if (showTimer) clearTimeout(showTimer)
      showTimer = null
      visible.value = false
      currentId = null
    }
  }

  async function cancel() {
    if (!currentId || cancelling.value) return
    cancelling.value = true
    try {
      await cancelPrepare(currentId)
    } catch (e) {
      console.error('Cancel prepare failed:', e)
    }
  }

  return { visible, label, phase, done, total, cancelling, onProgress, run, cancel }
})
