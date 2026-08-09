<script setup lang="ts">
import { useThemeStore } from '@/stores/theme'

const props = defineProps<{
  split: boolean
}>()

const emit = defineEmits<{
  connect: []
  bookmarks: []
  upload: []
  uploadFolder: []
  download: []
  refresh: []
  newFolder: []
  terminal: []
  'toggle-split': []
}>()

const theme = useThemeStore()
</script>

<template>
  <div class="toolbar">
    <button class="btn" @click="emit('connect')">
      <span>⚡</span> Connect
    </button>
    <button class="btn" @click="emit('bookmarks')">
      <span>⭐</span> Bookmarks
    </button>
    <button class="btn" @click="emit('upload')">
      <span>⬆</span> Upload
    </button>
    <button class="btn" @click="emit('uploadFolder')">
      <span>⬆</span> Upload Folder
    </button>
    <button class="btn" @click="emit('download')">
      <span>⬇</span> Download
    </button>
    <button class="btn" @click="emit('refresh')">
      <span>🔄</span> Refresh
    </button>
    <button class="btn" @click="emit('newFolder')">
      <span>📁</span> New Folder
    </button>
    <button class="btn" @click="emit('terminal')">
      <span>🖥</span> Terminal
    </button>
    <span class="spacer" />
    <button
      class="btn icon-btn"
      :title="props.split ? 'Close split view (Ctrl+\\)' : 'Split view (Ctrl+\\)'"
      @click="emit('toggle-split')"
    >
      ◫
    </button>
    <button
      class="btn icon-btn"
      :title="theme.theme === 'dark' ? 'Light mode' : 'Dark mode'"
      @click="theme.toggle()"
    >
      {{ theme.theme === 'dark' ? '☀' : '☾' }}
    </button>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 12px;
  background: var(--bg-primary);
  border-bottom: 1px solid var(--border);
}

.spacer {
  flex: 1;
}

.btn {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  background: var(--surface);
  border: 1px solid var(--text-disabled);
  border-radius: 4px;
  color: var(--text-primary);
  font-size: 12px;
  cursor: pointer;
  font-family: inherit;
  flex-shrink: 0;
}

.btn:hover {
  background: var(--text-disabled);
}

.icon-btn {
  padding: 4px 8px;
  font-size: 14px;
}
</style>
