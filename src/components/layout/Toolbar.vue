<script setup lang="ts">
const emit = defineEmits<{
  connect: []
  bookmarks: []
  upload: []
  download: []
  refresh: []
  newFolder: []
}>()
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
    <button class="btn" @click="emit('download')">
      <span>⬇</span> Download
    </button>
    <button class="btn" @click="emit('refresh')">
      <span>🔄</span> Refresh
    </button>
    <button class="btn" @click="emit('newFolder')">
      <span>📁</span> New Folder
    </button>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  gap: 4px;
  padding: 6px 12px;
  background: #1e1e2e;
  border-bottom: 1px solid #313244;
}

.btn {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  color: #cdd6f4;
  font-size: 12px;
  cursor: pointer;
}

.btn:hover {
  background: #45475a;
}
</style>
