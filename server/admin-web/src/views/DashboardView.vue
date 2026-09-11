<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Collection, User, Download, Star } from '@element-plus/icons-vue'
import { api } from '../api'

interface Trend { date: string; identities: number; filters: number; downloads: number }
interface Dashboard {
  activeIdentities: number; revokedIdentities: number; activeFilters: number; disabledFilters: number
  totalLikes: number; totalDownloads: number; latestVersion: string; trend: Trend[]
}
const loading = ref(true)
const data = ref<Dashboard | null>(null)
const maxTrend = computed(() => Math.max(1, ...(data.value?.trend || []).map(item => item.identities + item.filters + item.downloads)))
const height = (value: number) => `${Math.max(value ? 4 : 0, (value / maxTrend.value) * 150)}px`
onMounted(async () => { try { data.value = await api('/admin/api/dashboard') } finally { loading.value = false } })
</script>

<template>
  <div class="page-stack" v-loading="loading">
    <div class="page-title-row"><div><h1>仪表盘</h1><p>快速了解服务中的身份、关键词和使用情况。</p></div><el-tag effect="plain">当前版本 {{ data?.latestVersion || '—' }}</el-tag></div>
    <section class="metric-grid">
      <article class="panel metric-card"><span class="metric-label">有效访问身份</span><div class="metric-value">{{ data?.activeIdentities ?? 0 }}</div><div class="metric-note">已撤销 {{ data?.revokedIdentities ?? 0 }}</div><div class="metric-icon"><el-icon><User /></el-icon></div></article>
      <article class="panel metric-card"><span class="metric-label">正常关键词</span><div class="metric-value">{{ data?.activeFilters ?? 0 }}</div><div class="metric-note">已停用 {{ data?.disabledFilters ?? 0 }}</div><div class="metric-icon"><el-icon><Collection /></el-icon></div></article>
      <article class="panel metric-card"><span class="metric-label">累计下载</span><div class="metric-value">{{ data?.totalDownloads ?? 0 }}</div><div class="metric-note">按身份与版本去重</div><div class="metric-icon"><el-icon><Download /></el-icon></div></article>
      <article class="panel metric-card"><span class="metric-label">累计点赞</span><div class="metric-value">{{ data?.totalLikes ?? 0 }}</div><div class="metric-note">当前全部关键词</div><div class="metric-icon"><el-icon><Star /></el-icon></div></article>
    </section>
    <section class="panel">
      <header class="panel-header"><div><h2>最近 14 天</h2><span class="cell-sub">新增身份、关键词与下载趋势</span></div><div class="toolbar"><el-tag size="small" color="#3157d5" effect="dark">身份</el-tag><el-tag size="small" color="#7c93ea" effect="dark">关键词</el-tag><el-tag size="small" color="#c7d2f7">下载</el-tag></div></header>
      <div class="panel-body">
        <div class="chart-bars" aria-label="最近十四天活动趋势">
          <div v-for="item in data?.trend || []" :key="item.date" class="chart-day" :title="`${item.date}：身份 ${item.identities}，关键词 ${item.filters}，下载 ${item.downloads}`">
            <div class="chart-stack"><i class="chart-bar downloads" :style="{height:height(item.downloads)}" /><i class="chart-bar filters" :style="{height:height(item.filters)}" /><i class="chart-bar" :style="{height:height(item.identities)}" /></div>
            <span>{{ item.date.slice(5) }}</span>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>
