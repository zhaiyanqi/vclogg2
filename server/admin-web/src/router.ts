import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'
import DashboardView from './views/DashboardView.vue'
import FiltersView from './views/FiltersView.vue'
import IdentitiesView from './views/IdentitiesView.vue'
import AdministratorsView from './views/AdministratorsView.vue'
import RolesView from './views/RolesView.vue'
import SettingsView from './views/SettingsView.vue'
import AuditView from './views/AuditView.vue'
import AccountView from './views/AccountView.vue'

export const adminRoutes: RouteRecordRaw[] = [
  { path: '/', redirect: '/dashboard' },
  { path: '/dashboard', name: 'dashboard', component: DashboardView, meta: { title: '仪表盘', permission: 'dashboard.read', group: 'overview' } },
  { path: '/filters', name: 'filters', component: FiltersView, meta: { title: '云端关键词', permission: 'filters.read', group: 'content' } },
  { path: '/identities', name: 'identities', component: IdentitiesView, meta: { title: '访问身份', permission: 'identities.read', group: 'content' } },
  { path: '/administrators', name: 'administrators', component: AdministratorsView, meta: { title: '管理员', permission: 'admins.read', group: 'system' } },
  { path: '/roles', name: 'roles', component: RolesView, meta: { title: '角色权限', permission: 'roles.read', group: 'system' } },
  { path: '/settings', name: 'settings', component: SettingsView, meta: { title: '版本与备份', permissions: ['settings.version.read', 'backup.create'], group: 'system' } },
  { path: '/audit', name: 'audit', component: AuditView, meta: { title: '操作审计', permission: 'audit.read', group: 'system' } },
  { path: '/account', name: 'account', component: AccountView, meta: { title: '账户安全', group: 'account' } },
  { path: '/:pathMatch(.*)*', redirect: '/' }
]

export const router = createRouter({ history: createWebHistory('/admin/'), routes: adminRoutes })
