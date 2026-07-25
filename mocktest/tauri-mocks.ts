// Mocks for Tauri APIs used by RemotePanel when running in a plain browser.
export function getCurrentWebview() {
  return {
    onDragDropEvent: async () => () => {},
  }
}

export const open = async () => null
export const save = async () => null
export const ask = async () => true
export const message = async () => {}
export const listen = async () => () => {}
export const invoke = async () => {
  throw new Error('invoke not available in mock')
}
