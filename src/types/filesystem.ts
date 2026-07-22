export interface FileEntry {
  name: string
  path: string
  isDir: boolean
  size: number
  modified: number
  permissions: string | null
}

export interface FilePreview {
  isText: boolean
  content: string | null
  truncated: boolean
}
