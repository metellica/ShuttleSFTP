export interface ConnectionProfile {
  id: string
  name: string
  host: string
  port: number
  username: string
  authMethod: 'password' | 'key' | 'agent'
  privateKeyPath?: string
  defaultRemotePath?: string
  /** Present only when the user opted in to saving the password. */
  password?: string
  /** Present only when the user opted in (key passphrase). */
  passphrase?: string
}

export interface ConnectParams {
  host: string
  port: number
  username: string
  auth: PasswordAuth | KeyAuth | AgentAuth
}

export interface PasswordAuth {
  type: 'password'
  password: string
}

export interface KeyAuth {
  type: 'key'
  key_path: string
  passphrase: string | null
}

export interface AgentAuth {
  type: 'agent'
}

export interface SshHostEntry {
  name: string
  hostname: string | null
  port: number | null
  user: string | null
  identityFile: string | null
}

export type SessionKind = 'ssh' | 'container' | 'pod'

export type RuntimeKind = 'docker' | 'nerdctl' | 'crictl' | 'kubectl'

/** A running container listed by the connect dialog picker. */
export interface ContainerInfo {
  id: string
  name: string
  image: string
  state: string
  runtime: RuntimeKind
  /** K8s pod this container belongs to (crictl listings). */
  pod?: string
}

/** A pod listed by the K8s picker. */
export interface PodInfo {
  name: string
  namespace: string
  node: string | null
  phase: string
  containers: string[]
}

export interface ContainerConnectSpec {
  runtime: RuntimeKind
  containerId: string
  name?: string
  /** Reuse the SSH connection of an existing session (remote engine). */
  viaSessionId?: string
  /** Or open a dedicated SSH connection (bookmark reconnects). */
  via?: ConnectParams
  /** Try direct rootfs access through the host before exec+shell. */
  preferRootfs?: boolean
}

export interface PodConnectSpec {
  context?: string
  namespace: string
  pod: string
  container?: string
  /** Where kubectl runs: an existing session's host, or local when unset. */
  viaSessionId?: string
  via?: ConnectParams
}

/** Everything App needs to record about a fresh connection. */
export interface ConnectedMeta {
  kind: SessionKind
  /** SSH leg params (null for local containers/pods). */
  params: ConnectParams | null
  containerSpec?: ContainerConnectSpec
  podSpec?: PodConnectSpec
  initialPath?: string
}

export interface Bookmark {
  id: string
  alias: string
  host: string
  port: number
  username: string
  authMethod: 'password' | 'key' | 'agent'
  privateKeyPath?: string
  /** Present only when captured from a password connection. */
  password?: string
  /** Present only when captured from a key connection with passphrase. */
  passphrase?: string
  /** Remote directory this bookmark opens. */
  path: string
  /** Endpoint type; missing means classic SSH bookmark. */
  kind?: SessionKind
  /** Container target (kind === 'container'). */
  container?: {
    runtime: RuntimeKind
    containerId: string
    name?: string
  }
  /** Pod target (kind === 'pod'). */
  pod?: {
    context?: string
    namespace: string
    pod: string
    container?: string
  }
}
