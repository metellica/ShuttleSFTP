import { createApp, h, ref } from 'vue'

interface PromptOptions {
  /** Pre-filled input value. */
  defaultValue?: string
  /** Mask input (for passwords). */
  password?: boolean
  /** Confirm button label. */
  okLabel?: string
}

/**
 * Promise-based replacement for window.prompt(), which is not supported
 * in WKWebView on macOS (it always returns null there).
 * Resolves with the entered string, or null if cancelled.
 */
export function promptText(title: string, options: PromptOptions = {}): Promise<string | null> {
  return new Promise((resolve) => {
    const host = document.createElement('div')
    document.body.appendChild(host)

    const app = createApp({
      setup() {
        const value = ref(options.defaultValue ?? '')
        const inputRef = ref<HTMLInputElement | null>(null)

        function done(result: string | null) {
          app.unmount()
          host.remove()
          resolve(result)
        }

        return () =>
          h(
            'div',
            {
              class: 'sp-overlay',
              onClick: (e: MouseEvent) => {
                if (e.target === e.currentTarget) done(null)
              },
              onKeydown: (e: KeyboardEvent) => {
                if (e.key === 'Escape') done(null)
              },
            },
            [
              h('div', { class: 'sp-dialog' }, [
                h('div', { class: 'sp-title' }, title),
                h('input', {
                  ref: (el: any) => {
                    inputRef.value = el
                    // Focus once mounted
                    if (el) setTimeout(() => el.focus(), 0)
                  },
                  class: 'sp-input',
                  type: options.password ? 'password' : 'text',
                  value: value.value,
                  onInput: (e: Event) => {
                    value.value = (e.target as HTMLInputElement).value
                  },
                  onKeydown: (e: KeyboardEvent) => {
                    if (e.key === 'Enter') done(value.value)
                  },
                }),
                h('div', { class: 'sp-actions' }, [
                  h('button', { class: 'sp-btn sp-cancel', onClick: () => done(null) }, 'Cancel'),
                  h(
                    'button',
                    { class: 'sp-btn sp-ok', onClick: () => done(value.value) },
                    options.okLabel ?? 'OK'
                  ),
                ]),
              ]),
            ]
          )
      },
    })

    app.mount(host)
  })
}

// Inject dialog styles once (matches app theme).
const STYLE_ID = 'sp-prompt-style'
if (!document.getElementById(STYLE_ID)) {
  const style = document.createElement('style')
  style.id = STYLE_ID
  style.textContent = `
.sp-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
.sp-dialog {
  background: #1e1e2e;
  border: 1px solid #45475a;
  border-radius: 8px;
  padding: 20px;
  width: 380px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.sp-title {
  color: #cdd6f4;
  font-size: 14px;
  font-weight: 600;
  word-break: break-all;
}
.sp-input {
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 10px;
  color: #cdd6f4;
  font-size: 13px;
  outline: none;
}
.sp-input:focus {
  border-color: #89b4fa;
}
.sp-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.sp-btn {
  padding: 5px 14px;
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
  border: 1px solid #45475a;
}
.sp-cancel {
  background: #313244;
  color: #cdd6f4;
}
.sp-ok {
  background: #89b4fa;
  color: #1e1e2e;
  border-color: #89b4fa;
}
.sp-ok:hover {
  background: #74c7ec;
}
`
  document.head.appendChild(style)
}
