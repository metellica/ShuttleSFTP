import { invoke } from '@tauri-apps/api/core'
import {
  readText as clipboardPluginReadText,
  writeText as clipboardPluginWriteText,
} from '@tauri-apps/plugin-clipboard-manager'
import type {
  ConnectParams,
  SshHostEntry,
  ConnectionProfile,
  Bookmark,
} from '@/types/connection'
import type { FileEntry, FilePreview } from '@/types/filesystem'
import type { TransferTask } from '@/types/transfer'

// Connection
export const sshConnect = (params: ConnectParams) =>
  invoke<string>('connect', { params })

/** Session on this machine: local files + /@containers + /@pods. */
export const connectLocal = () => invoke<string>('connect_local')

export const sshDisconnect = (sessionId: string) =>
  invoke<void>('disconnect', { sessionId })

// Filesystem
export const listDir = (sessionId: string, path: string) =>
  invoke<FileEntry[]>('list_dir', { sessionId, path })

export const mkDir = (sessionId: string, path: string) =>
  invoke<void>('mkdir', { sessionId, path })

export const removeEntry = (sessionId: string, path: string, isDir: boolean, prepareId?: string) =>
  invoke<void>('remove', { sessionId, path, isDir, prepareId: prepareId ?? null })

export const renameEntry = (sessionId: string, oldPath: string, newPath: string) =>
  invoke<void>('rename', { sessionId, oldPath, newPath })

export const previewFile = (sessionId: string, path: string, full = false) =>
  invoke<FilePreview>('preview_file', { sessionId, path, full })

export const saveFileContent = (sessionId: string, path: string, content: string) =>
  invoke<void>('save_file', { sessionId, path, content })

// Transfer
export const uploadFiles = (
  sessionId: string,
  localPaths: string[],
  remoteDir: string,
  prepareId?: string
) => invoke<string[]>('upload', { sessionId, localPaths, remoteDir, prepareId: prepareId ?? null })

export const downloadFiles = (
  sessionId: string,
  remotePaths: string[],
  localDir: string,
  prepareId?: string
) =>
  invoke<string[]>('download', { sessionId, remotePaths, localDir, prepareId: prepareId ?? null })

export const downloadFileAs = (
  sessionId: string,
  remotePath: string,
  localPath: string,
  prepareId?: string
) =>
  invoke<string[]>('download_as', {
    sessionId,
    remotePath,
    localPath,
    prepareId: prepareId ?? null,
  })

export const transferRemote = (
  srcSessionId: string,
  srcPaths: string[],
  dstSessionId: string,
  dstDir: string,
  prepareId?: string
) =>
  invoke<string[]>('transfer_remote', {
    srcSessionId,
    srcPaths,
    dstSessionId,
    dstDir,
    prepareId: prepareId ?? null,
  })

/** Abort an in-flight preparation (queueing/deleting) by its id. */
export const cancelPrepare = (prepareId: string) =>
  invoke<void>('cancel_prepare', { prepareId })

export const cancelTransfer = (taskId: string, deleteLocal = false) =>
  invoke<void>('cancel_transfer', { taskId, deleteLocal })

export const cancelAllTransfers = (deleteLocal = false) =>
  invoke<void>('cancel_all_transfers', { deleteLocal })

export const cancelTransferGroup = (groupId: string, deleteLocal = false) =>
  invoke<void>('cancel_transfer_group', { groupId, deleteLocal })

export const pauseTransfer = (taskId: string) =>
  invoke<void>('pause_transfer', { taskId })

export const pauseAllTransfers = () =>
  invoke<string[]>('pause_all_transfers')

export const pauseTransferGroup = (groupId: string) =>
  invoke<string[]>('pause_transfer_group', { groupId })

export const resumeTransfer = (taskId: string, sessionId?: string) =>
  invoke<void>('resume_transfer', { taskId, sessionId: sessionId ?? null })

export const resumeAllTransfers = () =>
  invoke<string[]>('resume_all_transfers')

export const resumeTransferGroup = (groupId: string, sessionId?: string) =>
  invoke<string[]>('resume_transfer_group', { groupId, sessionId: sessionId ?? null })

export const clearFinishedTransfers = () =>
  invoke<void>('clear_finished_transfers')

export const showInFolder = (path: string) =>
  invoke<void>('show_in_folder', { path })

export const listTransfers = () =>
  invoke<TransferTask[]>('list_transfers')

// Terminal
export const terminalReserve = (terminalId: string) =>
  invoke<string>('terminal_reserve', { terminalId })

export const terminalOpen = (
  terminalId: string,
  terminalToken: string,
  sessionId: string,
  path: string,
  cols: number,
  rows: number
) =>
  invoke<void>('terminal_open', {
    request: { terminalId, terminalToken, sessionId, path, cols, rows },
  })

export const terminalInput = (terminalId: string, terminalToken: string, data: string) =>
  invoke<void>('terminal_input', { terminalId, terminalToken, data })

export const terminalResize = (
  terminalId: string,
  terminalToken: string,
  cols: number,
  rows: number
) => invoke<void>('terminal_resize', { terminalId, terminalToken, cols, rows })

export const terminalClose = (terminalId: string, terminalToken: string) =>
  invoke<void>('terminal_close', { terminalId, terminalToken })

// --- System clipboard (text) -------------------------------------------------
// The WebView's own clipboard API needs a user gesture and a permission
// prompt to read, so the terminal goes through the plugin instead.

export const clipboardWriteText = (text: string) => clipboardPluginWriteText(text)

/** Empty string when the clipboard holds no text (an image, files, …). */
export const clipboardReadText = async (): Promise<string> => {
  try {
    return (await clipboardPluginReadText()) ?? ''
  } catch {
    return ''
  }
}

// Config
export const loadSshConfig = () =>
  invoke<SshHostEntry[]>('load_ssh_config')

export const listImportedSshHosts = () =>
  invoke<string[]>('list_imported_ssh_hosts')

export const setImportedSshHosts = (names: string[]) =>
  invoke<void>('set_imported_ssh_hosts', { names })

export const listProfiles = () =>
  invoke<ConnectionProfile[]>('list_profiles')

export const saveProfile = (profile: ConnectionProfile) =>
  invoke<void>('save_profile', { profile })

export const deleteProfile = (profileId: string) =>
  invoke<void>('delete_profile', { profileId })

// Bookmarks
export const listBookmarks = () =>
  invoke<Bookmark[]>('list_bookmarks')

export const saveBookmark = (bookmark: Bookmark) =>
  invoke<void>('save_bookmark', { bookmark })

export const deleteBookmark = (bookmarkId: string) =>
  invoke<void>('delete_bookmark', { bookmarkId })
