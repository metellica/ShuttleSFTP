<script setup lang="ts">
import { useTabsStore } from '@/stores/tabs'

const emit = defineEmits<{ 'new-tab': [] }>()
const tabsStore = useTabsStore()
</script>

<template>
  <div class="tab-bar">
    <div
      v-for="tab in tabsStore.tabs"
      :key="tab.id"
      class="tab"
      :class="{ active: tab.id === tabsStore.activeTabId }"
      @click="tabsStore.setActiveTab(tab.id)"
    >
      <span class="tab-status" :class="tab.status" />
      <span class="tab-label">{{ tab.label }}</span>
      <button class="tab-close" @click.stop="tabsStore.closeTab(tab.id)">×</button>
    </div>
    <button class="tab-add" @click="emit('new-tab')">+</button>
  </div>
</template>

<style scoped>
.tab-bar {
  display: flex;
  background: #181825;
  border-bottom: 1px solid #313244;
  height: 36px;
  align-items: stretch;
  overflow-x: auto;
  user-select: none;
}

.tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  cursor: pointer;
  border-right: 1px solid #313244;
  font-size: 12px;
  color: #a6adc8;
  min-width: 120px;
  max-width: 200px;
}

.tab.active {
  background: #1e1e2e;
  color: #cdd6f4;
}

.tab:hover {
  background: #242438;
}

.tab-status {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #6c7086;
}

.tab-status.connected { background: #a6e3a1; }
.tab-status.connecting { background: #f9e2af; }
.tab-status.error { background: #f38ba8; }

.tab-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tab-close {
  background: none;
  border: none;
  color: #6c7086;
  cursor: pointer;
  font-size: 16px;
  line-height: 1;
  padding: 2px;
}

.tab-close:hover { color: #f38ba8; }

.tab-add {
  background: none;
  border: none;
  color: #a6adc8;
  cursor: pointer;
  font-size: 18px;
  padding: 0 12px;
}

.tab-add:hover { color: #cdd6f4; }
</style>
