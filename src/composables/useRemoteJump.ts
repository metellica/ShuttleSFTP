import {
  sshConnect,
  connectLocal,
  listProfiles,
  listBookmarks,
  loadSshConfig,
} from '@/composables/useTauri'
import { promptText } from '@/composables/usePrompt'
import { serverRefOf, useTabsStore } from '@/stores/tabs'
import type { ConnectParams, JumpHost, ServerRef } from '@/types/connection'

interface SavedCredentials {
  host: string
  port: number
  username: string
  authMethod: 'password' | 'key' | 'agent'
  privateKeyPath?: string
  password?: string
  passphrase?: string
  jumpHosts?: JumpHost[]
}

function paramsFrom(src: SavedCredentials, password?: string): ConnectParams {
  const base = {
    host: src.host,
    port: src.port,
    username: src.username,
    jumpHosts: src.jumpHosts?.map((jump) => ({ ...jump })),
  }
  if (src.authMethod === 'key') {
    return {
      ...base,
      auth: { type: 'key', key_path: src.privateKeyPath ?? '', passphrase: src.passphrase ?? null },
    }
  }
  if (src.authMethod === 'agent') return { ...base, auth: { type: 'agent' } }
  return { ...base, auth: { type: 'password', password: password ?? src.password ?? '' } }
}

/**
 * Connection details for a remembered folder's server, from whatever the
 * user has saved: profiles, bookmarks, then `~/.ssh/config`. A saved
 * password-less password login asks for the password instead of failing.
 */
async function resolveParams(target: ServerRef): Promise<ConnectParams | null> {
  const [profiles, bookmarks] = await Promise.all([
    listProfiles().catch(() => []),
    listBookmarks().catch(() => []),
  ])
  const saved: SavedCredentials[] = [...profiles, ...bookmarks].filter(
    (s) =>
      s.host === target.host &&
      s.port === target.port &&
      s.username === target.username &&
      (s as { kind?: string }).kind !== 'local'
  )
  const complete = saved.find((s) => s.authMethod !== 'password' || s.password)
  if (complete) return paramsFrom(complete)
  if (saved[0]) {
    const input = await promptText(`Password for ${target.username}@${target.host}:`, {
      password: true,
    })
    return input === null ? null : paramsFrom(saved[0], input)
  }

  const hosts = await loadSshConfig().catch(() => [])
  const entry = hosts.find(
    (h) =>
      (h.hostname ?? h.name) === target.host &&
      (h.port ?? 22) === target.port &&
      (h.user ?? '') === target.username
  )
  if (!entry) return null
  return paramsFrom({
    host: target.host,
    port: target.port,
    username: target.username,
    authMethod: entry.identityFile ? 'key' : 'agent',
    privateKeyPath: entry.identityFile ?? undefined,
    jumpHosts: entry.jumpHosts,
  })
}

/**
 * Open a remembered folder: switch to the tab already connected to that
 * server, or connect with saved credentials and open one. Resolves with
 * an error message when it cannot, or null on success (also when the
 * user cancels a password prompt).
 */
export async function jumpToRecent(target: ServerRef, path: string): Promise<string | null> {
  const tabsStore = useTabsStore()

  for (const pane of tabsStore.panes) {
    const live = pane.tabs.find(
      (t) => t.status === 'connected' && t.sessionId && serverRefOf(t)?.serverKey === target.serverKey
    )
    if (live) {
      tabsStore.setActiveTab(live.id)
      tabsStore.updateTab(live.id, { currentPath: path })
      return null
    }
  }

  let params: ConnectParams | null = null
  let sessionId: string
  try {
    if (target.kind === 'local') {
      sessionId = await connectLocal()
    } else {
      params = await resolveParams(target)
      if (!params) {
        return `No saved connection for ${target.label}. Connect to it once with Connect… or save a profile.`
      }
      sessionId = await sshConnect(params)
    }
  } catch (e) {
    return `Connection to ${target.label} failed: ${e}`
  }

  const tab =
    tabsStore.activeTab && tabsStore.activeTab.status === 'disconnected'
      ? tabsStore.activeTab
      : tabsStore.addTab()
  tabsStore.updateTab(tab.id, {
    sessionId,
    label: target.label,
    status: 'connected',
    currentPath: path,
    kind: target.kind,
    connectParams: params,
  })
  return null
}
