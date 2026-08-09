import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'

/**
 * Row scale for the remote file browser.
 *
 * Presets cover the common cases; the range in between is free so the
 * size can be dialled in with Ctrl+wheel or the slider — a fixed set of
 * three steps is not enough for everyone's eyesight or display DPI.
 */
export const MIN_ROW_SCALE = 0.75
export const MAX_ROW_SCALE = 2.5
export const ROW_SCALE_STEP = 0.08

/** Neither pane may be squeezed past this share of the window. */
export const MIN_SPLIT_RATIO = 0.15
export const MAX_SPLIT_RATIO = 0.85

export interface RowPreset {
  id: 'small' | 'medium' | 'large'
  label: string
  scale: number
}

export const ROW_PRESETS: RowPreset[] = [
  { id: 'small', label: 'Small', scale: 0.85 },
  { id: 'medium', label: 'Medium', scale: 1 },
  { id: 'large', label: 'Large', scale: 1.4 },
]

/** Width of every details-list column, in unscaled pixels. */
export interface ColumnWidths {
  name: number
  size: number
  permissions: number
  modified: number
}

/**
 * Column widths are stored unscaled: the listing multiplies them by the
 * row scale, so a dragged width keeps its proportions when the rows zoom.
 */
export const DEFAULT_COLUMN_WIDTHS: ColumnWidths = {
  name: 280,
  size: 90,
  permissions: 110,
  modified: 170,
}

export type ColumnKey = keyof ColumnWidths

export const COLUMN_KEYS = Object.keys(DEFAULT_COLUMN_WIDTHS) as ColumnKey[]

/** Narrow enough to be useful, wide enough that the header stays grabbable. */
export const MIN_COLUMN_WIDTH = 56
export const MAX_COLUMN_WIDTH = 1200

const STORAGE_KEY = 'shuttle-sftp:view'

function clamp(value: number): number {
  return Math.min(MAX_ROW_SCALE, Math.max(MIN_ROW_SCALE, value))
}

function clampWidth(value: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback
  return Math.round(Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, value)))
}

interface StoredView {
  rowScale: number
  columnWidths: ColumnWidths
  stretchName: boolean
  splitRatio: number
}

/** A setting missing from an older payload falls back to its default. */
function load(): StoredView {
  const fallback: StoredView = {
    rowScale: 1,
    columnWidths: { ...DEFAULT_COLUMN_WIDTHS },
    stretchName: true,
    splitRatio: 0.5,
  }
  const raw = localStorage.getItem(STORAGE_KEY)
  if (!raw) return fallback
  try {
    const parsed = JSON.parse(raw) as {
      rowScale?: unknown
      columnWidths?: Partial<ColumnWidths>
      stretchName?: unknown
      splitRatio?: unknown
    }
    if (typeof parsed.rowScale === 'number') fallback.rowScale = clamp(parsed.rowScale)
    for (const key of COLUMN_KEYS) {
      fallback.columnWidths[key] = clampWidth(
        Number(parsed.columnWidths?.[key]),
        DEFAULT_COLUMN_WIDTHS[key]
      )
    }
    if (typeof parsed.stretchName === 'boolean') fallback.stretchName = parsed.stretchName
    if (typeof parsed.splitRatio === 'number') fallback.splitRatio = clampRatio(parsed.splitRatio)
    return fallback
  } catch {
    return fallback
  }
}

function clampRatio(value: number): number {
  if (!Number.isFinite(value)) return 0.5
  return Math.min(MAX_SPLIT_RATIO, Math.max(MIN_SPLIT_RATIO, value))
}

export const useViewSettingsStore = defineStore('viewSettings', () => {
  const stored = load()
  const rowScale = ref(stored.rowScale)
  const columnWidths = ref<ColumnWidths>(stored.columnWidths)
  // Until a divider is dragged the name column fills whatever is left, which
  // is the layout most people expect from a fresh window.
  const stretchName = ref(stored.stretchName)
  const splitRatio = ref(0.5)

  function save() {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        rowScale: rowScale.value,
        columnWidths: { ...columnWidths.value },
        stretchName: stretchName.value,
        splitRatio: splitRatio.value,
      })
    )
  }

  watch([rowScale, stretchName, splitRatio], save)
  watch(columnWidths, save, { deep: true })

  /** The preset the current scale corresponds to, if any. */
  const activePreset = computed(
    () => ROW_PRESETS.find((p) => Math.abs(p.scale - rowScale.value) < 0.005)?.id ?? null
  )

  const percent = computed(() => Math.round(rowScale.value * 100))

  function setScale(value: number) {
    rowScale.value = clamp(value)
  }

  function setPreset(id: RowPreset['id']) {
    const preset = ROW_PRESETS.find((p) => p.id === id)
    if (preset) rowScale.value = preset.scale
  }

  /** Ctrl+wheel / Ctrl+= / Ctrl+- adjustment. */
  function nudge(direction: 1 | -1) {
    // Round to the step grid so repeated nudges don't drift to values
    // like 1.0399999999999998.
    const next = Math.round((rowScale.value + direction * ROW_SCALE_STEP) * 100) / 100
    rowScale.value = clamp(next)
  }

  function reset() {
    rowScale.value = 1
  }

  /** Dragged header divider, in unscaled pixels. */
  function setColumnWidth(key: ColumnKey, value: number) {
    // A name column that has been given a width of its own can no longer
    // stretch, or dragging it narrower would be undone by the fill.
    if (key === 'name') stretchName.value = false
    columnWidths.value[key] = clampWidth(value, DEFAULT_COLUMN_WIDTHS[key])
  }

  function resetColumnWidth(key: ColumnKey) {
    columnWidths.value[key] = DEFAULT_COLUMN_WIDTHS[key]
    if (key === 'name') stretchName.value = true
  }

  function resetColumnWidths() {
    columnWidths.value = { ...DEFAULT_COLUMN_WIDTHS }
    stretchName.value = true
  }

  /** Dragged splitter position, as the left pane's share of the width. */
  function setSplitRatio(value: number) {
    splitRatio.value = clampRatio(value)
  }

  return {
    rowScale,
    columnWidths,
    stretchName,
    splitRatio,
    activePreset,
    percent,
    setScale,
    setPreset,
    setColumnWidth,
    resetColumnWidth,
    resetColumnWidths,
    setSplitRatio,
    nudge,
    reset,
  }
})
