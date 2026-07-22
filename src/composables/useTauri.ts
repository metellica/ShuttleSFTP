import { invoke } from '@tauri-apps/api/core'
import type { ConnectParams, SshHostEntry, ConnectionProfile } from '@/types/connection'
import type { FileEntry } from '@/types/filesystem'
import type { TransferTask } from '@/types/transfer'

// Connection
export const sshConnect = (params: ConnectParams) =>
  invoke<string>('connect', { params })

export const sshDisconnect = (sessionId: string) =>
  invoke<void>('disconnect', { sessionId })

// Filesystem
export const listDir = (sessionId: string, path: string) =>
  invoke<FileEntry[]>('list_dir', { sessionId, path })

export const mkDir = (sessionId: string, path: string) =>
  invoke<void>('mkdir', { sessionId, path })

export const removeEntry = (sessionId: string, path: string, isDir: boolean) =>
  invoke<void>('remove', { sessionId, path, isDir })

export const renameEntry = (sessionId: string, oldPath: string, newPath: string) =>
  invoke<void>('rename', { sessionId, oldPath, newPath })

// Transfer
export const uploadFiles = (sessionId: string, localPaths: string[], remoteDir: string) =>
  invoke<string[]>('upload', { sessionId, localPaths, remoteDir })

export const downloadFiles = (sessionId: string, remotePaths: string[], localDir: string) =>
  invoke<string[]>('download', { sessionId, remotePaths, localDir })

export const downloadFileAs = (sessionId: string, remotePath: string, localPath: string) =>
  invoke<string>('download_as', { sessionId, remotePath, localPath })

export const cancelTransfer = (taskId: string) =>
  invoke<void>('cancel_transfer', { taskId })

export const listTransfers = () =>
  invoke<TransferTask[]>('list_transfers')

// Config
export const loadSshConfig = () =>
  invoke<SshHostEntry[]>('load_ssh_config')

export const listProfiles = () =>
  invoke<ConnectionProfile[]>('list_profiles')

export const saveProfile = (profile: ConnectionProfile) =>
  invoke<void>('save_profile', { profile })

export const deleteProfile = (profileId: string) =>
  invoke<void>('delete_profile', { profileId })
