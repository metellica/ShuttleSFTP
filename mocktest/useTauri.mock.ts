// Mock of composables/useTauri for headless UI debugging (no Tauri backend).
import type { FileEntry, FilePreview } from '@/types/filesystem'

const now = Math.floor(Date.now() / 1000)

function file(dir: string, name: string, size = 1234): FileEntry {
  return {
    name,
    path: dir === '/' ? `/${name}` : `${dir}/${name}`,
    isDir: false,
    size,
    modified: now,
    permissions: '-rw-r--r--',
  }
}
function folder(dir: string, name: string): FileEntry {
  return {
    name,
    path: dir === '/' ? `/${name}` : `${dir}/${name}`,
    isDir: true,
    size: 0,
    modified: now,
    permissions: 'drwxr-xr-x',
  }
}

const fs: Record<string, FileEntry[]> = {}
fs['/'] = [folder('/', 'data'), ...Array.from({ length: 30 }, (_, i) => folder('/', `root${i}`))]
fs['/data'] = [
  folder('/data', 'jenkins'),
  ...Array.from({ length: 10 }, (_, i) => folder('/data', `d${i}`)),
]
fs['/data/jenkins'] = [
  folder('/data/jenkins', 'release'),
  ...Array.from({ length: 50 }, (_, i) => folder('/data/jenkins', `build-${i + 32}`)),
]
fs['/data/jenkins/release'] = [
  folder('/data/jenkins/release', '79'),
  folder('/data/jenkins/release', '80'),
  folder('/data/jenkins/release', '81'),
]
fs['/rel'] = [folder('/rel', '79'), folder('/rel', '80'), folder('/rel', '81')]
fs['/rel/81'] = [
  file('/rel/81', 'common.sh', 13000),
  file('/rel/81', 'install.sh', 5500),
  file('/rel/81', 'start.sh', 2100),
]
fs['/data/jenkins/release/81'] = [
  file('/data/jenkins/release/81', '.env', 591),
  file('/data/jenkins/release/81', '.env.example', 560),
  file('/data/jenkins/release/81', 'common.sh', 13000),
  file('/data/jenkins/release/81', 'docker-compose.yml', 1200),
  file('/data/jenkins/release/81', 'image.tar.zst', 9900000000),
  file('/data/jenkins/release/81', 'install-compose.sh', 6900),
  file('/data/jenkins/release/81', 'install.sh', 5500),
  file('/data/jenkins/release/81', 'restart-compose.sh', 502),
  file('/data/jenkins/release/81', 'restart.sh', 515),
  file('/data/jenkins/release/81', 'start-compose.sh', 1100),
  file('/data/jenkins/release/81', 'start.sh', 2100),
  file('/data/jenkins/release/81', 'stop-compose.sh', 541),
  file('/data/jenkins/release/81', 'stop.sh', 589),
]

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms))

export const sshConnect = async () => 'mock-session'
export const sshDisconnect = async () => {}

export const listDir = async (_s: string, path: string): Promise<FileEntry[]> => {
  await delay(80)
  const entries = fs[path]
  if (!entries) throw new Error(`No such directory: ${path}`)
  return [...entries]
}

export const mkDir = async () => {}
export const removeEntry = async () => {}
export const renameEntry = async () => {}

export const previewFile = async (_s: string, path: string): Promise<FilePreview> => {
  await delay(120)
  const lines = Array.from({ length: 60 }, (_, i) => `# line ${i + 1} of mock file ${path}`)
  return { isText: true, content: lines.join('\n'), truncated: false }
}

export const saveFileContent = async () => {}
export const uploadFiles = async () => []
export const downloadFiles = async () => []
export const downloadFileAs = async () => ''
export const cancelTransfer = async () => {}
export const cancelAllTransfers = async () => {}
export const cancelTransferGroup = async () => {}
export const pauseTransfer = async () => {}
export const pauseAllTransfers = async () => []
export const resumeTransfer = async () => {}
export const resumeAllTransfers = async () => []
export const clearFinishedTransfers = async () => {}
export const showInFolder = async () => {}
export const listTransfers = async () => []
export const loadSshConfig = async () => []
export const listProfiles = async () => []
export const saveProfile = async () => {}
export const deleteProfile = async () => {}
export const listBookmarks = async () => []
export const saveBookmark = async () => {}
export const deleteBookmark = async () => {}

// Container / pod / cross-session additions
export const connectContainer = async () => 'mock-container-session'
export const connectPod = async () => 'mock-pod-session'
export const listContainers = async () => [
  {
    id: 'abc123def456',
    name: 'redis',
    image: 'redis:7',
    state: 'Up 3 hours',
    runtime: 'docker' as const,
  },
  {
    id: '789xyz000111',
    name: 'nginx-7d4-app',
    image: 'nginx:1.27',
    state: 'running',
    runtime: 'crictl' as const,
    pod: 'nginx-7d4',
  },
]
export const listKubeContexts = async () => ['prod', 'staging']
export const listKubeNamespaces = async () => ['default', 'kube-system']
export const listKubePods = async (namespace: string) => [
  {
    name: 'nginx-7d4',
    namespace,
    node: 'node-1',
    phase: 'Running',
    containers: ['nginx', 'sidecar'],
  },
]
export const transferRemote = async () => []
