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

export type SessionKind = 'ssh' | 'local'

/** Everything App needs to record about a fresh connection. */
export interface ConnectedMeta {
  kind: SessionKind
  /** SSH leg params (null for local sessions). */
  params: ConnectParams | null
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
  /** Remote directory this bookmark opens (may point into /@containers or /@pods). */
  path: string
  /** Endpoint type; missing means classic SSH bookmark. */
  kind?: SessionKind
}
