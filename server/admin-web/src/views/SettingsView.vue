<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ElMessage, type UploadFile, type UploadInstance } from 'element-plus'
import { Download, Refresh, UploadFilled } from '@element-plus/icons-vue'
import { api, jsonBody, uploadForm } from '../api'
import { useSessionStore } from '../store'

interface PublishedRelease {
  version: string
  installerName: string
  installerSize: number
  publishedAt: number
}

interface ReleaseStatus {
  enabled: boolean
  maxUploadBytes: number
  release?: PublishedRelease
  warning?: string
}

const session = useSessionStore()
const loading = ref(true)
const saving = ref(false)
const backingUp = ref(false)
const publishing = ref(false)
const publishingVCLogg2 = ref(false)
const uploadProgress = ref(0)
const uploadProgressVCLogg2 = ref(0)
const version = ref('')
const updatedAt = ref(0)
const releaseStatus = ref<ReleaseStatus | null>(null)
const releaseStatusVCLogg2 = ref<ReleaseStatus | null>(null)
const releaseFile = ref<File | null>(null)
const releaseFileVCLogg2 = ref<File | null>(null)
const releaseUpload = ref<UploadInstance>()
const releaseUploadVCLogg2 = ref<UploadInstance>()

function formatBytes(value: number) {
  if (value >= 1024 * 1024 * 1024) return `${(value / 1024 / 1024 / 1024).toFixed(2)} GiB`
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MiB`
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`
  return `${value} B`
}

async function load() {
  loading.value = true
  try {
    if (session.can('settings.version.read')) {
      const [versionData, status, statusVCLogg2] = await Promise.all([
        api<{ latestVersion: string; updatedAt: number }>('/admin/api/settings/version'),
        api<ReleaseStatus>('/admin/api/settings/releases/vclogg'),
        api<ReleaseStatus>('/admin/api/settings/releases/vclogg2')
      ])
      version.value = versionData.latestVersion
      updatedAt.value = versionData.updatedAt
      releaseStatus.value = status
      releaseStatusVCLogg2.value = statusVCLogg2
    }
  } finally {
    loading.value = false
  }
}

async function save() {
  saving.value = true
  try {
    const data = await api<{ latestVersion: string; updatedAt: number }>('/admin/api/settings/version', { method: 'PUT', body: jsonBody({ latestVersion: version.value.trim() }) })
    updatedAt.value = data.updatedAt
    ElMessage.success('客户端版本已更新')
  } finally {
    saving.value = false
  }
}

function selectRelease(file: UploadFile) {
  releaseFile.value = file.raw || null
}

function removeRelease() {
  releaseFile.value = null
}

function selectVCLogg2Release(file: UploadFile) {
  releaseFileVCLogg2.value = file.raw || null
}

function removeVCLogg2Release() {
  releaseFileVCLogg2.value = null
}

async function publishRelease() {
  if (!releaseFile.value) return
  publishing.value = true
  uploadProgress.value = 0
  try {
    const body = new FormData()
    body.append('package', releaseFile.value)
    const result = await uploadForm<{ latestVersion: string; updatedAt: number; release: PublishedRelease }>('/admin/api/settings/releases/vclogg', body, (progress) => { uploadProgress.value = progress })
    version.value = result.latestVersion
    updatedAt.value = result.updatedAt
    releaseStatus.value = { enabled: true, maxUploadBytes: releaseStatus.value?.maxUploadBytes || 0, release: result.release }
    releaseFile.value = null
    releaseUpload.value?.clearFiles()
    ElMessage.success(`VCLogg ${result.latestVersion} 已发布`)
  } finally {
    publishing.value = false
  }
}

async function publishVCLogg2Release() {
  if (!releaseFileVCLogg2.value) return
  publishingVCLogg2.value = true
  uploadProgressVCLogg2.value = 0
  try {
    const body = new FormData()
    body.append('package', releaseFileVCLogg2.value)
    const result = await uploadForm<{ latestVersion: string; updatedAt: number; release: PublishedRelease }>('/admin/api/settings/releases/vclogg2', body, (progress) => { uploadProgressVCLogg2.value = progress })
    releaseStatusVCLogg2.value = { enabled: true, maxUploadBytes: releaseStatusVCLogg2.value?.maxUploadBytes || 0, release: result.release }
    releaseFileVCLogg2.value = null
    releaseUploadVCLogg2.value?.clearFiles()
    ElMessage.success(`VCLogg2 ${result.latestVersion} 已发布`)
  } finally {
    publishingVCLogg2.value = false
  }
}

async function backup() {
  backingUp.value = true
  try {
    const response = await api<Response>('/admin/api/backup', { method: 'POST' })
    const blob = await response.blob(), url = URL.createObjectURL(blob), link = document.createElement('a')
    link.href = url
    link.download = `vclogg-backup-${new Date().toISOString().slice(0, 10)}.db`
    link.click()
    URL.revokeObjectURL(url)
    ElMessage.success('数据库备份已生成')
  } finally {
    backingUp.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="page-stack" v-loading="loading">
    <div class="page-title-row"><div><h1>版本与备份</h1><p>在同一个后台分别发布 VCLogg 和 VCLogg2，并下载一致性的 SQLite 在线备份。</p></div></div>
    <section v-if="session.can('settings.version.read')" class="panel">
      <header class="panel-header"><div><h2>VCLogg 软件发布</h2><span class="cell-sub">发布到 /updates/win-x64/，并自动同步 VCLogg 版本标记</span></div></header>
      <div class="panel-body">
        <el-alert v-if="releaseStatus && !releaseStatus.enabled" title="服务端配置已关闭软件更新，启用 updates.enabled 后才能发布。" type="warning" :closable="false" show-icon />
        <el-alert v-else-if="releaseStatus?.warning" :title="releaseStatus.warning" type="warning" :closable="false" show-icon />
        <div v-if="releaseStatus?.release" class="release-summary">
          <div><span class="cell-sub">当前已发布</span><strong>VCLogg {{ releaseStatus.release.version }}</strong></div>
          <div><span class="cell-sub">安装包</span><strong>{{ releaseStatus.release.installerName }}</strong></div>
          <div><span class="cell-sub">大小</span><strong>{{ formatBytes(releaseStatus.release.installerSize) }}</strong></div>
          <div><span class="cell-sub">发行时间</span><strong>{{ new Date(releaseStatus.release.publishedAt).toLocaleString() }}</strong></div>
        </div>
        <el-upload
          v-if="session.can('settings.version.update') && releaseStatus?.enabled"
          ref="releaseUpload"
          class="release-upload"
          drag
          accept=".zip,application/zip"
          :auto-upload="false"
          :limit="1"
          :on-change="selectRelease"
          :on-remove="removeRelease"
        >
          <el-icon class="el-icon--upload"><UploadFilled /></el-icon>
          <div class="el-upload__text">拖入发行 ZIP，或<em>选择文件</em></div>
          <template #tip><div class="el-upload__tip">ZIP 中只能包含 latest.yml、匹配版本的 x64 安装包和 .blockmap，最大 {{ formatBytes(releaseStatus.maxUploadBytes) }}</div></template>
        </el-upload>
        <div v-if="session.can('settings.version.update') && releaseStatus?.enabled" class="release-actions">
          <el-button type="primary" :loading="publishing" :disabled="!releaseFile" @click="publishRelease">{{ publishing ? `正在上传 ${uploadProgress}%` : '校验并发布' }}</el-button>
          <span class="cell-sub">安装包先完整落盘，latest.yml 最后替换；已发布的历史差分文件会保留。</span>
        </div>
      </div>
    </section>
    <section v-if="session.can('settings.version.read')" class="panel">
      <header class="panel-header"><div><h2>VCLogg2 软件发布</h2><span class="cell-sub">发布到 /updates-vclogg2/win-x64/，文件与 VCLogg 完全分开</span></div></header>
      <div class="panel-body">
        <el-alert v-if="releaseStatusVCLogg2 && !releaseStatusVCLogg2.enabled" title="服务端配置已关闭软件更新，启用 updates.enabled 后才能发布。" type="warning" :closable="false" show-icon />
        <el-alert v-else-if="releaseStatusVCLogg2?.warning" :title="releaseStatusVCLogg2.warning" type="warning" :closable="false" show-icon />
        <div v-if="releaseStatusVCLogg2?.release" class="release-summary">
          <div><span class="cell-sub">当前已发布</span><strong>VCLogg2 {{ releaseStatusVCLogg2.release.version }}</strong></div>
          <div><span class="cell-sub">便携更新包</span><strong>{{ releaseStatusVCLogg2.release.installerName }}</strong></div>
          <div><span class="cell-sub">大小</span><strong>{{ formatBytes(releaseStatusVCLogg2.release.installerSize) }}</strong></div>
          <div><span class="cell-sub">发行时间</span><strong>{{ new Date(releaseStatusVCLogg2.release.publishedAt).toLocaleString() }}</strong></div>
        </div>
        <el-upload
          v-if="session.can('settings.version.update') && releaseStatusVCLogg2?.enabled"
          ref="releaseUploadVCLogg2"
          class="release-upload"
          drag
          accept=".zip,application/zip"
          :auto-upload="false"
          :limit="1"
          :on-change="selectVCLogg2Release"
          :on-remove="removeVCLogg2Release"
        >
          <el-icon class="el-icon--upload"><UploadFilled /></el-icon>
          <div class="el-upload__text">拖入 VCLogg2 发行 ZIP，或<em>选择文件</em></div>
          <template #tip><div class="el-upload__tip">ZIP 中只能包含 latest.json、匹配版本的 vclogg2-*-win-x64.zip 和 .blockmap.json，最大 {{ formatBytes(releaseStatusVCLogg2.maxUploadBytes) }}</div></template>
        </el-upload>
        <div v-if="session.can('settings.version.update') && releaseStatusVCLogg2?.enabled" class="release-actions">
          <el-button type="primary" :loading="publishingVCLogg2" :disabled="!releaseFileVCLogg2" @click="publishVCLogg2Release">{{ publishingVCLogg2 ? `正在上传 ${uploadProgressVCLogg2}%` : '校验并发布 VCLogg2' }}</el-button>
          <span class="cell-sub">更新包先完整落盘，latest.json 最后替换；已发布的历史分块文件会保留。</span>
        </div>
      </div>
    </section>
    <section v-if="session.can('settings.version.read')" class="panel">
      <header class="panel-header"><div><h2>VCLogg 版本标记</h2><span class="cell-sub">通常由 VCLogg 软件发布自动同步，也可只修改版本提示</span></div></header>
      <div class="panel-body"><el-form label-position="top" style="max-width:520px"><el-form-item label="最新语义化版本"><div class="toolbar" style="width:100%"><el-input v-model="version" placeholder="0.8.4" style="flex:1"/><el-button v-if="session.can('settings.version.update')" type="primary" :loading="saving" @click="save">保存版本</el-button></div></el-form-item><p class="cell-sub">最后更新时间：{{ updatedAt ? new Date(updatedAt).toLocaleString() : '尚未设置' }}</p></el-form></div>
    </section>
    <section class="panel">
      <header class="panel-header"><div><h2>数据备份</h2><span class="cell-sub">备份包含管理员、角色、身份、关键词、会话和审计记录</span></div></header>
      <div class="panel-body"><el-alert title="服务运行期间不要直接复制 WAL 数据库文件。此操作使用 SQLite 在线备份 API 生成一致快照。" type="info" :closable="false" show-icon/><div style="margin-top:18px"><el-button v-if="session.can('backup.create')" type="primary" plain :icon="Download" :loading="backingUp" @click="backup">生成并下载备份</el-button><el-button :icon="Refresh" @click="load">刷新状态</el-button></div></div>
    </section>
  </div>
</template>

<style scoped>
.release-summary { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:12px; margin-bottom:18px; }
.release-summary > div { min-width:0; padding:14px; border:1px solid var(--el-border-color-lighter); border-radius:8px; background:var(--el-fill-color-extra-light); }
.release-summary span,.release-summary strong { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.release-summary strong { margin-top:6px; font-size:14px; }
.release-upload { max-width:680px; }
.release-actions { display:flex; align-items:center; gap:12px; margin-top:16px; }
@media (max-width:900px) { .release-summary { grid-template-columns:repeat(2,minmax(0,1fr)); } .release-actions { align-items:flex-start; flex-direction:column; } }
</style>
