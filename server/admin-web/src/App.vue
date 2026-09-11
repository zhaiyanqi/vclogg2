<script setup lang="ts">
import { onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { useSessionStore } from './store'
import LoginView from './views/LoginView.vue'
import AdminLayout from './components/AdminLayout.vue'

const session = useSessionStore()
onMounted(async () => {
  try { await session.load() } catch (error) { ElMessage.error((error as Error).message) }
})
</script>

<template>
  <div v-if="!session.ready" class="boot-screen" aria-label="正在加载">
    <div class="brand-mark">V</div><span>正在准备管理中心…</span>
  </div>
  <LoginView v-else-if="!session.session" />
  <AdminLayout v-else />
</template>
