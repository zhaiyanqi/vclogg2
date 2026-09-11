<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import {
  DataAnalysis, Collection, User, UserFilled, Lock, Setting,
  DocumentChecked, SwitchButton, Fold, Expand, ArrowDown
} from '@element-plus/icons-vue'
import { adminRoutes } from '../router'
import { useSessionStore } from '../store'

const session = useSessionStore()
const route = useRoute()
const router = useRouter()
const collapsed = ref(false)
const mobileOpen = ref(false)

const iconByName: Record<string, unknown> = {
  dashboard: DataAnalysis, filters: Collection, identities: User,
  administrators: UserFilled, roles: Lock, settings: Setting,
  audit: DocumentChecked, account: Lock
}

function canAccessRoute(item: (typeof adminRoutes)[number]) {
  const permissions = item.meta?.permissions as string[] | undefined
  return permissions
    ? permissions.some(permission => session.can(permission))
    : session.can(item.meta?.permission as string | undefined)
}

const navigation = computed(() => adminRoutes.filter(item =>
  item.name && canAccessRoute(item)
))
const groups = computed(() => [
  { id: 'overview', label: '概览', items: navigation.value.filter(item => item.meta?.group === 'overview') },
  { id: 'content', label: '内容管理', items: navigation.value.filter(item => item.meta?.group === 'content') },
  { id: 'system', label: '系统管理', items: navigation.value.filter(item => item.meta?.group === 'system') }
].filter(group => group.items.length))

watch(() => route.fullPath, () => { mobileOpen.value = false }, { immediate: true })
watch([() => session.session, () => route.meta.permission, () => route.meta.permissions], () => {
  const permission = route.meta.permission as string | undefined
  const permissions = route.meta.permissions as string[] | undefined
  const allowed = permissions
    ? permissions.some(item => session.can(item))
    : session.can(permission)
  if (session.session && !allowed) {
    const first = navigation.value[0]
    if (first) router.replace({ name: first.name })
  }
}, { immediate: true })

async function logout() {
  try { await session.logout(); await router.replace('/') }
  catch (error) { ElMessage.error((error as Error).message) }
}
</script>

<template>
  <div class="admin-shell" :class="{ 'is-collapsed': collapsed, 'mobile-open': mobileOpen }">
    <div class="mobile-scrim" @click="mobileOpen = false" />
    <aside class="sidebar">
      <div class="sidebar-brand">
        <div class="brand-mark">V</div>
        <div class="brand-copy"><strong>VCLogg</strong><span>管理中心</span></div>
      </div>
      <nav class="sidebar-nav" aria-label="主导航">
        <section v-for="group in groups" :key="group.id" class="nav-group">
          <p class="nav-label">{{ group.label }}</p>
          <RouterLink v-for="item in group.items" :key="String(item.name)" :to="{ name: item.name }" class="nav-item">
            <el-icon><component :is="iconByName[String(item.name)]" /></el-icon>
            <span>{{ item.meta?.title }}</span>
          </RouterLink>
        </section>
      </nav>
      <button class="collapse-button" type="button" @click="collapsed = !collapsed" :aria-label="collapsed ? '展开侧栏' : '收起侧栏'">
        <el-icon><Expand v-if="collapsed" /><Fold v-else /></el-icon><span>{{ collapsed ? '展开' : '收起侧栏' }}</span>
      </button>
    </aside>
    <section class="workspace">
      <header class="topbar">
        <button class="mobile-menu" type="button" @click="mobileOpen = true" aria-label="打开菜单"><el-icon><Expand /></el-icon></button>
        <div class="page-heading"><span>VCLogg /</span><strong>{{ route.meta.title }}</strong></div>
        <el-dropdown trigger="click">
          <button class="account-trigger" type="button">
            <span class="account-avatar">{{ session.session?.username.slice(0, 1).toUpperCase() }}</span>
            <span class="account-name">{{ session.session?.username }}</span>
            <el-icon><ArrowDown /></el-icon>
          </button>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="router.push('/account')"><el-icon><Lock /></el-icon>账户安全</el-dropdown-item>
              <el-dropdown-item divided @click="logout"><el-icon><SwitchButton /></el-icon>退出登录</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </header>
      <main class="page-canvas"><RouterView /></main>
    </section>
  </div>
</template>
