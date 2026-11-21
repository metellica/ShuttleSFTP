import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import * as api from '@/composables/useTauri'
import type { RecentDir } from '@/types/connection'

/** Folders shown per server before "+N more". */
const GROUP_PREVIEW = 6
/** A server untouched this long is folded away. */
const DORMANT_SECS = 14 * 86_400

export interface RecentGroup {
  serverKey: string
  label: string
  kind: RecentDir['kind']
  items: RecentDir[]
  total: number
  lastVisit: number
  dormant: boolean
}

/** Remembered folders across all servers, ranked by frecency. */
export const useRecentStore = defineStore('recent', () => {
  const entries = ref<RecentDir[]>([])

  /** Servers by how recently any of their folders was used, local last. */
  const groups = computed<RecentGroup[]>(() => {
    const byServer = new Map<string, RecentDir[]>()
    for (const entry of entries.value) {
      const list = byServer.get(entry.serverKey)
      if (list) list.push(entry)
      else byServer.set(entry.serverKey, [entry])
    }
    const now = Date.now() / 1000
    const out = [...byServer.entries()].map(([serverKey, list]) => {
      const lastVisit = Math.max(...list.map((e) => e.lastVisit))
      return {
        serverKey,
        label: list[0]!.label,
        kind: list[0]!.kind,
        // `entries` arrives ranked, so the head is already the best.
        items: list.slice(0, GROUP_PREVIEW),
        total: list.length,
        lastVisit,
        dormant: now - lastVisit > DORMANT_SECS,
      }
    })
    return out.sort((a, b) => b.lastVisit - a.lastVisit)
  })

  async function refresh() {
    entries.value = await api.listRecent()
  }

  async function togglePin(entry: RecentDir) {
    entries.value = await api.togglePinRecent(entry, entry.path)
  }

  async function remove(entry: RecentDir) {
    entries.value = await api.removeRecent(entry.serverKey, entry.path)
  }

  async function clear() {
    entries.value = await api.clearRecent()
  }

  return { entries, groups, refresh, togglePin, remove, clear }
})
