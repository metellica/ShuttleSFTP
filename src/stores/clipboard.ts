import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import {
  clipboardSeqNum,
  clipboardSupportsFiles,
  copyFilesToSystemClipboard,
  readSystemClipboardFiles,
} from '@/composables/useTauri'
import { usePrepareStore } from '@/stores/prepare'

export type PasteSource =
  | { kind: 'virtual'; sessionId: string; paths: string[]; label: string }
  | { kind: 'system'; paths: string[] }

/**
 * Files marked for copy, pasteable into any session's directory.
 *
 * Mixes two clipboards: an in-app "virtual" one (session + remote
 * paths, for efficient remote-to-remote transfers) and, on Windows, the
 * real system clipboard (CF_HDROP) — so files copied here can be
 * pasted into Explorer, and files copied in Explorer can be pasted
 * here (uploading them).
 *
 * `active` holds whichever was touched most recently and is what Paste
 * actually uses — including on a repeat Paste with no new copy in
 * between, which must repeat the same source rather than silently
 * falling back to older, unrelated state. The system clipboard is only
 * re-read (and `active` updated to it) when its sequence number has
 * moved past `lastSyncedSeq`, i.e. something wrote to it since we last
 * looked; a plain repeat Paste leaves `active` untouched.
 */
export const useClipboardStore = defineStore('clipboard', () => {
  const active = ref<PasteSource | null>(null)

  /** Whether this platform supports real files on the system clipboard. */
  const supportsFiles = ref(false)
  clipboardSupportsFiles()
    .then((v) => (supportsFiles.value = v))
    .catch(() => {})

  /** System clipboard sequence number as of the last time `active` was
   *  known to match it (either we just wrote to it, or we just read
   *  it into `active`); null until the first check. */
  const lastSyncedSeq = ref<number | null>(null)

  /** Tab the files were copied from ("From <tab>" in the menu); '' for
   *  a system-clipboard source (no single owning session). */
  const sourceLabel = computed(() => (active.value?.kind === 'virtual' ? active.value.label : ''))

  /** Number of files Paste would currently act on. */
  const pasteCount = computed(() => active.value?.paths.length ?? 0)

  /** Mark a selection for Paste: sets the virtual clipboard right away
   *  (so in-app Paste works instantly even if the step below fails or
   *  is slow) and, where supported, mirrors the same files onto the
   *  system clipboard — blocking on a "Preparing files…" spinner since
   *  it requires eagerly downloading them to a temp directory first. */
  async function copyFiles(fromSessionId: string, filePaths: string[], label: string) {
    active.value = { kind: 'virtual', sessionId: fromSessionId, paths: filePaths, label }
    if (!supportsFiles.value) return
    const prepareStore = usePrepareStore()
    try {
      const seq = await prepareStore.run('Preparing files for clipboard', (pid) =>
        copyFilesToSystemClipboard(fromSessionId, filePaths, pid)
      )
      if (seq !== undefined) lastSyncedSeq.value = seq
    } catch (e) {
      console.error('Copy to system clipboard failed:', e)
    }
  }

  /** Pull in a system-clipboard change since `lastSyncedSeq`, if any,
   *  updating `active` to it. A no-op (cheap: one seq-number IPC call)
   *  when nothing external has touched the clipboard since. */
  async function syncFromSystem() {
    if (!supportsFiles.value) return
    try {
      const seqNow = await clipboardSeqNum()
      if (seqNow === lastSyncedSeq.value) return
      const files = await readSystemClipboardFiles()
      lastSyncedSeq.value = seqNow
      // A format change to something other than files (e.g. plain
      // text) isn't a file copy — leave `active` as it was.
      if (files.length > 0) active.value = { kind: 'system', paths: files }
    } catch (e) {
      console.error('System clipboard check failed:', e)
    }
  }

  /** What a Paste should act on right now: whichever of the virtual or
   *  system clipboard was touched most recently. Returns null when
   *  there's nothing to paste. */
  async function resolvePasteSource(): Promise<PasteSource | null> {
    await syncFromSystem()
    return active.value
  }

  /** Refresh `active`/`pasteCount` from the system clipboard; call
   *  before showing Paste UI (context menu open) so enablement/label
   *  reflects an external copy made since we last checked. */
  async function refreshHasContent() {
    await syncFromSystem()
  }

  return {
    active,
    supportsFiles,
    sourceLabel,
    pasteCount,
    copyFiles,
    resolvePasteSource,
    refreshHasContent,
  }
})
