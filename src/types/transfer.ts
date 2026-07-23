export interface TransferTask {
  id: string
  sessionId: string
  direction: 'upload' | 'download'
  sourcePath: string
  destPath: string
  totalBytes: number
  transferredBytes: number
  status: 'queued' | 'active' | 'completed' | 'failed' | 'cancelled'
  /** Bytes per second, frontend-only (from progress events). */
  speed?: number
}

export interface TransferProgress {
  taskId: string
  transferredBytes: number
  totalBytes: number
  speed: number
}
