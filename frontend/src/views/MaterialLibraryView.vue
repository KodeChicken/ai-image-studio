<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { NButton, NInput, NModal, NSelect, NSwitch, NTag, useMessage } from 'naive-ui'
import { api } from '@/api/client'
import { useAuthStore } from '@/stores/auth'
import type { ImageAsset, MaterialCategory, MaterialTemplate } from '@/types/api'

const auth = useAuthStore()
const message = useMessage()
const categories = ref<MaterialCategory[]>([])
const templates = ref<MaterialTemplate[]>([])
const categoryId = ref<string | null>(null)
const loading = ref(false)
const categoryOpen = ref(false)
const categoryName = ref('')
const templateOpen = ref(false)
const title = ref('')
const uploadCategoryId = ref<string | null>(null)
const sourceUrl = ref('')
const sourceAuthor = ref('')
const license = ref('')
const published = ref(false)
const file = ref<File | null>(null)
const saving = ref(false)
const preview = ref<MaterialTemplate | null>(null)
const isAdmin = computed(() => auth.user?.role === 'admin')
const visibleTemplates = computed(() =>
  categoryId.value
    ? templates.value.filter((item) => item.categoryId === categoryId.value)
    : templates.value,
)

onMounted(load)

async function load() {
  loading.value = true
  try {
    const prefix = isAdmin.value ? '/api/v1/admin' : '/api/v1'
    ;[categories.value, templates.value] = await Promise.all([
      api<MaterialCategory[]>(`${prefix}/material-categories`),
      api<MaterialTemplate[]>(`${prefix}/material-templates`),
    ])
    if (categoryId.value && !categories.value.some((item) => item.id === categoryId.value)) {
      categoryId.value = null
    }
  } catch (error) {
    message.error(error instanceof Error ? error.message : '素材库加载失败')
  } finally {
    loading.value = false
  }
}

async function createCategory() {
  if (!categoryName.value.trim()) return message.error('请输入分类名称')
  try {
    const created = await api<MaterialCategory>('/api/v1/admin/material-categories', {
      method: 'POST',
      body: JSON.stringify({ name: categoryName.value }),
    })
    categoryName.value = ''
    categoryOpen.value = false
    await load()
    uploadCategoryId.value = created.id
    message.success('分类已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '分类创建失败')
  }
}

function chooseFile(event: Event) {
  file.value = (event.target as HTMLInputElement).files?.[0] ?? null
}

function openTemplateForm() {
  uploadCategoryId.value = categoryId.value ?? categories.value[0]?.id ?? null
  templateOpen.value = true
}

async function createTemplate() {
  if (!title.value.trim() || !uploadCategoryId.value || !file.value) {
    return message.error('请填写素材名称、分类并选择图片')
  }
  saving.value = true
  let assetId: string | null = null
  try {
    const form = new FormData()
    form.append('file', file.value)
    const asset = await api<ImageAsset>('/api/v1/image-assets/uploads', { method: 'POST', body: form })
    assetId = asset.id
    await api('/api/v1/admin/material-templates', {
      method: 'POST',
      body: JSON.stringify({
        categoryId: uploadCategoryId.value,
        title: title.value,
        imageAssetId: asset.id,
        sourceUrl: sourceUrl.value || null,
        sourceAuthor: sourceAuthor.value || null,
        license: license.value || null,
        published: published.value,
      }),
    })
    templateOpen.value = false
    title.value = sourceUrl.value = sourceAuthor.value = license.value = ''
    file.value = null
    published.value = false
    await load()
    message.success('素材已添加')
  } catch (error) {
    if (assetId) await api(`/api/v1/image-assets/${assetId}`, { method: 'DELETE' }).catch(() => undefined)
    message.error(error instanceof Error ? error.message : '素材添加失败')
  } finally {
    saving.value = false
  }
}

async function updateTemplate(item: MaterialTemplate, payload: { categoryId?: string; published?: boolean }) {
  try {
    await api(`/api/v1/admin/material-templates/${item.id}`, {
      method: 'PATCH',
      body: JSON.stringify(payload),
    })
    await load()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '素材更新失败')
    await load()
  }
}
</script>

<template>
  <div class="page material-page">
    <header class="page-header">
      <div><span class="eyebrow muted">MATERIAL LIBRARY</span><h1>图片素材库</h1><p>按行业和场景浏览可用的成品图片模板。</p></div>
      <div v-if="isAdmin" class="material-admin-actions">
        <n-button @click="categoryOpen = true">新建分类</n-button>
        <n-button type="primary" :disabled="!categories.length" @click="openTemplateForm">添加素材</n-button>
      </div>
    </header>

    <div class="material-categories" aria-label="素材分类">
      <button :class="{ active: categoryId === null }" type="button" @click="categoryId = null">全部 <small>{{ templates.length }}</small></button>
      <button v-for="item in categories" :key="item.id" :class="{ active: categoryId === item.id }" type="button" @click="categoryId = item.id">
        {{ item.name }} <small>{{ item.itemCount }}</small>
      </button>
    </div>

    <div v-if="visibleTemplates.length" class="material-grid" :aria-busy="loading">
      <article v-for="item in visibleTemplates" :key="item.id" class="material-card panel" :class="{ unpublished: !item.published }">
        <button type="button" class="material-image" :aria-label="`预览素材：${item.title}`" @click="preview = item">
          <img :src="item.imageUrl" :alt="item.title" loading="lazy" />
          <n-tag v-if="!item.published" size="small" type="warning">未上架</n-tag>
        </button>
        <div class="material-card-copy">
          <div><strong>{{ item.title }}</strong><small>{{ item.categoryName }} · {{ item.license || '授权信息未填写' }}</small></div>
          <div v-if="isAdmin" class="material-card-admin">
            <n-select size="small" :value="item.categoryId" :options="categories.map(category => ({ label: category.name, value: category.id }))" @update:value="value => updateTemplate(item, { categoryId: value })" />
            <n-switch :value="item.published" @update:value="value => updateTemplate(item, { published: value })"><template #checked>已上架</template><template #unchecked>未上架</template></n-switch>
          </div>
        </div>
      </article>
    </div>
    <div v-else class="large-empty panel"><span>▧</span><h2>{{ loading ? '正在加载素材' : '这个分类还没有素材' }}</h2><p v-if="isAdmin && !loading">添加图片后即可选择上架。</p></div>
  </div>

  <n-modal v-model:show="categoryOpen" preset="card" title="新建素材分类" class="dialog-card">
    <div class="form-stack"><label>分类名称<n-input v-model:value="categoryName" maxlength="64" @keyup.enter="createCategory" /></label><n-button type="primary" @click="createCategory">创建分类</n-button></div>
  </n-modal>

  <n-modal v-model:show="templateOpen" preset="card" title="添加图片素材" class="dialog-card">
    <div class="form-stack">
      <label>素材名称<n-input v-model:value="title" maxlength="128" /></label>
      <label>所属分类<n-select v-model:value="uploadCategoryId" :options="categories.map(item => ({ label: item.name, value: item.id }))" /></label>
      <label>图片文件<input class="native-file-input" type="file" accept="image/png,image/jpeg,image/webp,image/gif" @change="chooseFile" /></label>
      <label>来源链接（选填）<n-input v-model:value="sourceUrl" placeholder="https://" /></label>
      <label>作者或来源方（选填）<n-input v-model:value="sourceAuthor" /></label>
      <label>许可说明（选填）<n-input v-model:value="license" placeholder="例如：CC0 1.0、自有版权" /></label>
      <label class="material-publish-row">立即上架<n-switch v-model:value="published" /></label>
      <n-button type="primary" :loading="saving" @click="createTemplate">保存素材</n-button>
    </div>
  </n-modal>

  <n-modal :show="Boolean(preview)" preset="card" class="material-preview" :title="preview?.title" @update:show="value => { if (!value) preview = null }">
    <img v-if="preview" :src="preview.imageUrl" :alt="preview.title" />
    <p v-if="preview?.sourceAuthor || preview?.license">{{ preview.sourceAuthor || '来源作者未填写' }} · {{ preview.license || '许可未填写' }}</p>
    <a v-if="preview?.sourceUrl" :href="preview.sourceUrl" target="_blank" rel="noreferrer">查看素材来源</a>
  </n-modal>
</template>

<style scoped>
.material-admin-actions, .material-card-admin { display: flex; align-items: center; gap: 8px; }
.material-categories { display: flex; overflow-x: auto; gap: 9px; margin-bottom: 22px; padding-bottom: 4px; }
.material-categories button { flex: 0 0 auto; padding: 9px 13px; border: 1px solid var(--border); border-radius: 999px; cursor: pointer; color: var(--muted); background: var(--panel); }
.material-categories button.active { border-color: var(--brand); color: var(--brand); background: var(--brand-soft); }
.material-categories small { margin-left: 5px; opacity: .7; }
.material-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 18px; }
.material-card { overflow: hidden; }
.material-card.unpublished { opacity: .72; }
.material-image { position: relative; display: block; width: 100%; padding: 0; overflow: hidden; border: 0; cursor: zoom-in; background: var(--panel-2); }
.material-image img { display: block; width: 100%; aspect-ratio: 2 / 3; object-fit: cover; transition: transform .25s ease; }
.material-image:hover img { transform: scale(1.02); }
.material-image .n-tag { position: absolute; top: 10px; right: 10px; }
.material-card-copy { display: grid; gap: 12px; padding: 14px; }
.material-card-copy strong, .material-card-copy small { display: block; }
.material-card-copy small { margin-top: 5px; color: var(--muted); font-size: 10px; }
.material-card-admin { justify-content: space-between; }
.material-card-admin .n-select { width: 128px; }
.native-file-input { padding: 10px; border: 1px solid var(--border); border-radius: 10px; color: var(--text); background: var(--panel-2); }
.material-publish-row { grid-template-columns: 1fr auto !important; align-items: center; }
.material-preview { width: min(680px, calc(100vw - 28px)); }
.material-preview img { display: block; max-width: 100%; max-height: 70vh; margin: auto; object-fit: contain; }
.material-preview p { margin: 14px 0 4px; color: var(--muted); font-size: 12px; }
.material-preview a { color: var(--brand); font-size: 12px; }
@media (max-width: 850px) { .material-admin-actions { width: 100%; } .material-admin-actions .n-button { flex: 1; } .material-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; } .material-card-admin { align-items: stretch; flex-direction: column; } .material-card-admin .n-select { width: 100%; } }
@media (max-width: 480px) { .material-grid { grid-template-columns: 1fr; } }
</style>
