<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import { Lock, User } from '@element-plus/icons-vue'
import { adminRoutes } from '../router'
import { useSessionStore } from '../store'

const session = useSessionStore()
const router = useRouter()
const username = ref('')
const password = ref('')
const loading = ref(false)

async function submit() {
  if (!username.value.trim() || !password.value) return
  loading.value = true
  try {
    await session.login(username.value.trim(), password.value)
    const first = adminRoutes.find(item => item.name && session.can(item.meta?.permission as string | undefined))
    await router.replace(first ? { name: first.name } : '/account')
  } catch (error) {
    ElMessage.error((error as Error).message)
  } finally { loading.value = false }
}
</script>

<template>
  <main class="login-screen">
    <section class="login-intro">
      <div class="login-brand"><div class="brand-mark">V</div><span>VCLogg</span></div>
      <div class="intro-copy">
        <p class="eyebrow">PORTABLE FILTER SERVICE</p>
        <h1>把每一次管理操作，<br />都留在清晰的边界内。</h1>
        <p>关键词、访问身份、角色权限与安全审计统一管理。数据仍保留在当前服务器的 SQLite 文件中。</p>
      </div>
      <div class="security-note"><span class="status-dot" /> 单进程运行 · 本地数据 · 权限隔离</div>
    </section>
    <section class="login-panel">
      <form class="login-card" @submit.prevent="submit">
        <div><p class="eyebrow">ADMIN CONSOLE</p><h2>欢迎回来</h2><p class="subtle">使用管理员账户继续</p></div>
        <label class="field-label">管理员账号</label>
        <el-input v-model="username" size="large" autocomplete="username" placeholder="请输入账号" :prefix-icon="User" />
        <div class="password-label"><label class="field-label">密码</label><span>至少 10 个字符</span></div>
        <el-input v-model="password" size="large" type="password" show-password autocomplete="current-password" placeholder="请输入密码" :prefix-icon="Lock" @keyup.enter="submit" />
        <el-button native-type="submit" type="primary" size="large" :loading="loading" :disabled="!username.trim() || !password">登录管理中心</el-button>
        <p class="login-help">管理员账户只能通过服务端命令或有权限的管理员创建。</p>
      </form>
    </section>
  </main>
</template>
