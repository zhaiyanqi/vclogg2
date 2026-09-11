<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Download } from '@element-plus/icons-vue'
import { api, ApiError, jsonBody } from '../api'
import { useSessionStore } from '../store'

interface FilterItem { id:string; status:string; revision:number; name:string; value:string; useRegex:boolean; note:string; collaborative:boolean; ownerName:string; likeCount:number; downloadCount:number; updatedAt:number }
const session = useSessionStore()
const loading = ref(false), items = ref<FilterItem[]>([]), total = ref(0), dialog = ref(false), saving = ref(false), exporting = ref(false)
const query = reactive({ q:'', status:'active', page:1, pageSize:30 })
const edit = reactive({ id:'', revision:0, name:'', value:'', note:'', useRegex:false })
async function load(){ loading.value=true; try { const p=new URLSearchParams(Object.entries(query).map(([k,v])=>[k,String(v)])); const data=await api<{items:FilterItem[];total:number}>(`/admin/api/filters?${p}`); items.value=data.items;total.value=data.total } finally { loading.value=false } }
function openEdit(item:FilterItem){ Object.assign(edit,{id:item.id,revision:item.revision,name:item.name,value:item.value,note:item.note,useRegex:item.useRegex});dialog.value=true }
async function save(){
  saving.value=true
  try {
    await api(`/admin/api/filters/${edit.id}`,{method:'PATCH',body:jsonBody({name:edit.name,value:edit.value,note:edit.note,useRegex:edit.useRegex,baseRevision:edit.revision})})
    dialog.value=false
    ElMessage.success('关键词已更新')
    await load()
  } catch (error) {
    if (error instanceof ApiError && error.code === 'revision_conflict') {
      await load()
      const current=items.value.find(item=>item.id===edit.id)
      if(current) edit.revision=current.revision
      ElMessage.warning('云端词条已有新修订，当前草稿已保留；请比较后再次保存或取消')
      return
    }
    throw error
  } finally { saving.value=false }
}
async function setStatus(item:FilterItem,status:string){ if(status==='deleted') await ElMessageBox.confirm(`确定删除“${item.name}”吗？`,'删除关键词',{type:'warning',confirmButtonText:'删除',cancelButtonText:'取消'});await api(`/admin/api/filters/${item.id}`,{method:'PATCH',body:jsonBody({status,baseRevision:item.revision})});ElMessage.success('状态已更新');await load() }
async function exportFilters(format:'json'|'csv'){
  exporting.value=true
  try {
    const p=new URLSearchParams({q:query.q,status:query.status,format})
    const response=await api<Response>(`/admin/api/filters/export?${p}`)
    const blob=await response.blob(),url=URL.createObjectURL(blob),link=document.createElement('a')
    const disposition=response.headers.get('content-disposition')||''
    link.href=url
    link.download=disposition.match(/filename="([^"]+)"/)?.[1]||`vclogg-filters.${format}`
    link.click()
    URL.revokeObjectURL(url)
    ElMessage.success(`已导出 ${format.toUpperCase()} 文件`)
  } finally { exporting.value=false }
}
onMounted(load)
</script>
<template><div class="page-stack"><div class="page-title-row"><div><h1>云端关键词</h1><p>审核共享内容，维护关键词状态和版本记录。</p></div></div><section class="panel"><header class="panel-header"><div class="toolbar"><el-input v-model="query.q" class="search-input" clearable placeholder="搜索名称、关键词、备注或分享者" @keyup.enter="query.page=1;load()"/><el-select v-model="query.status" style="width:130px" @change="query.page=1;load()"><el-option label="正常" value="active"/><el-option label="已停用" value="disabled"/><el-option label="已删除" value="deleted"/><el-option label="全部" value="all"/></el-select><el-button type="primary" @click="query.page=1;load()">查询</el-button><el-button v-if="session.can('filters.export')" :icon="Download" :loading="exporting" @click="exportFilters('json')">导出 JSON</el-button><el-button v-if="session.can('filters.export')" :icon="Download" :loading="exporting" @click="exportFilters('csv')">导出 CSV</el-button></div></header><el-table :data="items" v-loading="loading" style="width:100%"><el-table-column label="关键词" min-width="250"><template #default="{row}"><div class="cell-main">{{row.name}}</div><div class="cell-sub" :title="row.id">{{row.id.slice(0,8)}} · r{{row.revision}} · {{row.useRegex?'正则表达式':'普通文本'}}</div></template></el-table-column><el-table-column label="匹配内容" min-width="240" show-overflow-tooltip prop="value"/><el-table-column label="分享者" width="140" prop="ownerName"/><el-table-column label="统计" width="110"><template #default="{row}">♥ {{row.likeCount}}　↓ {{row.downloadCount}}</template></el-table-column><el-table-column label="状态" width="100"><template #default="{row}"><el-tag :type="row.status==='active'?'success':row.status==='disabled'?'warning':'info'" effect="light">{{row.status==='active'?'正常':row.status==='disabled'?'停用':'删除'}}</el-tag></template></el-table-column><el-table-column label="更新时间" width="170"><template #default="{row}">{{new Date(row.updatedAt).toLocaleString()}}</template></el-table-column><el-table-column label="操作" width="240" fixed="right"><template #default="{row}"><el-button v-if="session.can('filters.update')" link type="primary" @click="openEdit(row)">编辑</el-button><el-button v-if="session.can('filters.status') && row.status!=='active'" link type="success" @click="setStatus(row,'active')">恢复</el-button><el-button v-if="session.can('filters.status') && row.status==='active'" link type="warning" @click="setStatus(row,'disabled')">停用</el-button><el-button v-if="session.can('filters.status') && row.status!=='deleted'" link type="danger" @click="setStatus(row,'deleted')">删除</el-button></template></el-table-column><template #empty><div class="empty-state"><strong>没有符合条件的关键词</strong>调整搜索条件后再试。</div></template></el-table><footer class="table-footer"><el-pagination v-model:current-page="query.page" v-model:page-size="query.pageSize" layout="total, sizes, prev, pager, next" :total="total" :page-sizes="[20,30,50,100]" @change="load"/></footer></section><el-dialog v-model="dialog" title="编辑关键词" width="min(620px,92vw)"><el-form label-position="top"><el-form-item label="UUID"><el-input :model-value="edit.id" readonly/></el-form-item><el-form-item label="名称"><el-input v-model="edit.name" maxlength="100" show-word-limit/></el-form-item><el-form-item label="匹配内容"><el-input v-model="edit.value" type="textarea" :rows="4" maxlength="2000" show-word-limit/></el-form-item><el-form-item label="备注"><el-input v-model="edit.note" type="textarea" :rows="3" maxlength="1000" show-word-limit/></el-form-item><el-checkbox v-model="edit.useRegex">使用正则表达式</el-checkbox></el-form><template #footer><el-button @click="dialog=false">取消</el-button><el-button type="primary" :loading="saving" @click="save">保存更改</el-button></template></el-dialog></div></template>
