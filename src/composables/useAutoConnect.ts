import { sshConnect, connectLocal, listProfiles, listBookmarks } from '@/composables/useTauri'
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
 * For remote-to-remote copies both endpoints are connected.
 * Returns the (new or reused) primary session id, or null when a needed
 * connection could not be established.
 */
export async function connectForTransferTask(task: TransferTask): Promise<string | null> {
  if (!task.host) return null

  const primary = await ensureSessionFor(task.host, task.username ?? '', remoteDirOf(task))
  if (!primary) return null

  // Remote-to-remote copies also need the destination side alive.
  if (task.direction === 'remote' && task.destHost) {
    const destDir = task.destPath.slice(0, Math.max(task.destPath.lastIndexOf('/'), 1))
    const dest = await ensureSessionFor(task.destHost, task.destUsername ?? '', destDir)
    if (!dest) return null
  }
  return primary
}

/** A live session for host/user: reuse a connected tab, else auto-connect. */
async function ensureSessionFor(
  host: string,
  username: string,
  initialDir: string
): Promise<string | null> {
  const tabsStore = useTabsStore()

  // Reuse an already connected tab for this endpoint
  const live = tabsStore.tabs.find(
    (t) =>
      t.status === 'connected' &&
      t.sessionId &&
      (host === 'local'
        ? t.kind === 'local'
        : t.connectParams?.host === host &&
          (!username || t.connectParams?.username === username))
  )
  if (live?.sessionId) return live.sessionId

  if (host === 'local') {
    try {
      const sessionId = await connectLocal()
      openTabFor(sessionId, 'Local', initialDir, null, 'local')
      return sessionId
    } catch (e) {
      console.error('Cannot open local session:', e)
      return null
    }
  }

  let params: ConnectParams | null = null
  try {
    const profiles = await listProfiles()
    const profile = profiles.find(
      (p) => p.host === host && (!username || p.username === username)
    )
    if (profile) params = paramsFrom(profile)
    if (!params) {
      const bookmarks = await listBookmarks()
      const bookmark = bookmarks.find(
        (b) => b.host === host && (!username || b.username === username)
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
    openTabFor(sessionId, `${params.username}@${host}`, initialDir, params, 'ssh')
    return sessionId
  } catch (e) {
    console.error(`Auto-connect to ${params.username}@${host} failed:`, e)
    return null
  }
}

function openTabFor(
  sessionId: string,
  label: string,
  currentPath: string,
  connectParams: ConnectParams | null,
  kind: 'ssh' | 'local'
) {
  const tabsStore = useTabsStore()
  const tab =
    tabsStore.activeTab && tabsStore.activeTab.status === 'disconnected'
      ? tabsStore.activeTab
      : tabsStore.addTab()
  tabsStore.updateTab(tab.id, {
    sessionId,
    label,
    status: 'connected',
    currentPath,
    connectParams,
    kind,
  })
}
