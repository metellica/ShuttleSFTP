export interface TransferTask {
  id: string
  sessionId: string
  host?: string
  username?: string
  /** Set when this task is part of a directory transfer. */
  groupId?: string
  groupName?: string
  /** Path relative to the transferred directory root, '/'-separated. */
  relPath?: string
  /** Queue time in epoch milliseconds. */
  createdAt?: number
  direction: 'upload' | 'download'
  sourcePath: string
  destPath: string
  totalBytes: number
  transferredBytes: number
  status: 'queued' | 'active' | 'paused' | 'completed' | 'failed' | 'cancelled'
  /** Bytes per second, frontend-only (from progress events). */
  speed?: number
}

export interface TransferProgress {
  taskId: string
  transferredBytes: number
  totalBytes: number
  speed: number
}
