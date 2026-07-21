<script setup lang="ts">
import { useTabsStore } from '@/stores/tabs'
import TabBar from '@/components/layout/TabBar.vue'
import Toolbar from '@/components/layout/Toolbar.vue'
import ConnectDialog from '@/components/connection/ConnectDialog.vue'
import RemotePanel from '@/components/browser/RemotePanel.vue'
import TransferQueue from '@/components/transfer/TransferQueue.vue'
import { ref, onMounted } from 'vue'

const tabsStore = useTabsStore()
const showConnectDialog = ref(false)

onMounted(() => {
  if (tabsStore.tabs.length === 0) {
    tabsStore.addTab()
    showConnectDialog.value = true
  }
})

function onNewTab() {
  tabsStore.addTab()
  showConnectDialog.value = true
}

function onConnected(sessionId: string, label: string) {
  if (tabsStore.activeTab) {
    tabsStore.updateTab(tabsStore.activeTab.id, {
      sessionId,
      label,
      status: 'connected',
      currentPath: '/',
    })
  }
  showConnectDialog.value = false
}
</script>

<template>
  <div class="app-container">
    <TabBar @new-tab="onNewTab" />
    <Toolbar @connect="showConnectDialog = true" />
    <main class="main-content">
      <RemotePanel v-if="tabsStore.activeTab?.status === 'connected'" />
      <div v-else class="empty-state">
        <p>Click "Connect" or press the + tab to start a new SFTP session</p>
      </div>
    </main>
    <TransferQueue />
    <ConnectDialog
      v-if="showConnectDialog"
      @close="showConnectDialog = false"
      @connected="onConnected"
    />
  </div>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  overflow: hidden;
}
</style>

<style scoped>
.app-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #1e1e2e;
  color: #cdd6f4;
}

.main-content {
  flex: 1;
  overflow: hidden;
}

.empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #6c7086;
  font-size: 14px;
}
</style>
