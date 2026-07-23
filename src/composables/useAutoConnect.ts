import { sshConnect, listProfiles, listBookmarks } from '@/composables/useTauri'
import { useTabsStore } from '@/stores/tabs'
import type { ConnectParams } from '@/types/connection'
import type { TransferTask } from '@/types/transfer'

interface SavedCredentials {
  host: string
  port: number
  username: string
  authMethod: 'password' | 'key' | 'agent'
  privateKeyPath?: string
  password?: string
  passphrase?: string
}

function paramsFrom(src: SavedCredentials): ConnectParams | null {
  if (src.authMethod === 'password') {
    if (!src.password) return null
    return {
      host: src.host,
      port: src.port,
      username: src.username,
      auth: { type: 'password', password: src.password },
    }
  }
  if (src.authMethod === 'key') {
    if (!src.privateKeyPath) return null
    return {
      host: src.host,
      port: src.port,
      username: src.username,
      auth: { type: 'key', key_path: src.privateKeyPath, passphrase: src.passphrase ?? null },
    }
  }
  return null // agent auth is not implemented in the backend
}

/** The remote directory a transfer task operates in. */
function remoteDirOf(task: TransferTask): string {
  const remotePath = task.direction === 'upload' ? task.destPath : task.sourcePath
  const idx = remotePath.lastIndexOf('/')
  return idx > 0 ? remotePath.slice(0, idx) : '/'
}

/**
 * Auto-connect to the server a transfer task belongs to, using saved
 * profile/bookmark credentials, and open a tab for the new session.
 * Returns the new session id, or null when no usable credentials exist.
 */
export async function connectForTransferTask(task: TransferTask): Promise<string | null> {
  if (!task.host || !task.username) return null

  let params: ConnectParams | null = null
  try {
    const profiles = await listProfiles()
    const profile = profiles.find((p) => p.host === task.host && p.username === task.username)
    if (profile) params = paramsFrom(profile)
    if (!params) {
      const bookmarks = await listBookmarks()
      const bookmark = bookmarks.find(
        (b) => b.host === task.host && b.username === task.username
      )
      if (bookmark) params = paramsFrom(bookmark)
    }
  } catch (e) {
    console.error('Cannot load saved credentials:', e)
    return null
  }
  if (!params) return null

  try {
    const sessionId = await sshConnect(params)
    const tabsStore = useTabsStore()
    const tab =
      tabsStore.activeTab && tabsStore.activeTab.status === 'disconnected'
        ? tabsStore.activeTab
        : tabsStore.addTab()
    tabsStore.updateTab(tab.id, {
      sessionId,
      label: `${task.username}@${task.host}`,
      status: 'connected',
      currentPath: remoteDirOf(task),
      connectParams: params,
    })
    return sessionId
  } catch (e) {
    console.error(`Auto-connect to ${task.username}@${task.host} failed:`, e)
    return null
  }
}
