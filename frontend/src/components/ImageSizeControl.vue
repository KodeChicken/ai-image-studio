<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { NButton, NInputNumber, NRadioButton, NRadioGroup, NSelect } from 'naive-ui'
import {
  aspectRatioMatches,
  closestNumber,
  dependentEdgeOptions,
  imageAspectRatio,
  parseImageSize,
  type FixedImageEdge,
} from '@/lib/imageSizing'
import type { ParameterDefinition } from '@/types/api'

const customValue = '__custom_size__'
const props = defineProps<{
  aspectRatio: string | null
  size: string | null
  aspectDefinition?: ParameterDefinition
  sizeDefinition: ParameterDefinition
}>()
const emit = defineEmits<{
  'update:aspectRatio': [value: string]
  'update:size': [value: string]
}>()

const customOpen = ref(false)
const fixedEdge = ref<FixedImageEdge>('width')
const fixedValue = ref<number | null>(null)
const dependentValue = ref<number | null>(null)
const parsedSize = computed(() => parseImageSize(props.size))
const effectiveAspectRatio = computed(() => (
  parsedSize.value ? imageAspectRatio(parsedSize.value) : props.aspectRatio ?? 'auto'
))
const schemaAspectRatios = computed(() => props.aspectDefinition?.options ?? ['auto', '1:1', '16:9', '9:16', '3:2', '2:3'])
const aspectRatioOptions = computed(() => {
  const values = [...schemaAspectRatios.value]
  if (effectiveAspectRatio.value !== 'auto' && !values.includes(effectiveAspectRatio.value)) {
    values.push(effectiveAspectRatio.value)
  }
  return values.map((value) => ({
    label: value === 'auto'
      ? 'Auto（默认）'
      : schemaAspectRatios.value.includes(value) ? value : `${value}（自定义）`,
    value,
  }))
})
const resolutionOptions = computed(() => {
  const configured = props.sizeDefinition.options ?? ['auto']
  const selectedAspect = parsedSize.value ? effectiveAspectRatio.value : props.aspectRatio ?? 'auto'
  const values = configured.filter((value) => {
    const size = parseImageSize(value)
    return !size || selectedAspect === 'auto' || aspectRatioMatches(size, selectedAspect)
  })
  if (props.size && props.size !== 'auto' && !values.includes(props.size)) values.push(props.size)
  const options = values.map((value) => ({
    label: value === 'auto' ? 'Auto（默认）' : value,
    value,
  }))
  if (props.sizeDefinition.allow_custom === true) {
    options.push({ label: '自定义尺寸…', value: customValue })
  }
  return options
})
const dependentOptions = computed(() => {
  if (fixedValue.value === null) return []
  return dependentEdgeOptions(fixedValue.value, fixedEdge.value, props.sizeDefinition)
})
const dependentSelectOptions = computed(() => dependentOptions.value.map((value) => ({
  label: `${value} px`,
  value,
})))
const edgeMultiple = computed(() => Math.max(1, props.sizeDefinition.constraints?.edgeMultiple ?? 1))
const maxEdge = computed(() => props.sizeDefinition.constraints?.maxEdge ?? 4096)
const customSize = computed(() => {
  if (fixedValue.value === null || dependentValue.value === null) return null
  return fixedEdge.value === 'width'
    ? { width: fixedValue.value, height: dependentValue.value }
    : { width: dependentValue.value, height: fixedValue.value }
})

watch([fixedValue, fixedEdge], () => {
  if (!dependentOptions.value.length) {
    dependentValue.value = null
    return
  }
  if (dependentValue.value !== null && dependentOptions.value.includes(dependentValue.value)) return
  const current = parsedSize.value
  const preferred = current
    ? fixedEdge.value === 'width' ? current.height : current.width
    : fixedValue.value ?? dependentOptions.value[0]!
  dependentValue.value = closestNumber(dependentOptions.value, preferred)
})

function changeAspectRatio(value: string) {
  if (value === effectiveAspectRatio.value) return
  emit('update:aspectRatio', value)
  if (parsedSize.value && (value === 'auto' || !aspectRatioMatches(parsedSize.value, value))) {
    emit('update:size', 'auto')
  }
}

function changeResolution(value: string) {
  if (value === customValue) {
    openCustomSize()
    return
  }
  emit('update:size', value)
  const size = parseImageSize(value)
  if (!size) return
  const ratio = imageAspectRatio(size)
  emit('update:aspectRatio', schemaAspectRatios.value.includes(ratio) ? ratio : 'auto')
}

function openCustomSize() {
  const current = parsedSize.value ?? { width: 1024, height: 1024 }
  fixedEdge.value = 'width'
  fixedValue.value = current.width
  const options = dependentEdgeOptions(current.width, 'width', props.sizeDefinition)
  dependentValue.value = options.includes(current.height)
    ? current.height
    : closestNumber(options, current.height)
  customOpen.value = true
}

function changeFixedEdge(value: FixedImageEdge) {
  const current = customSize.value ?? parsedSize.value ?? { width: 1024, height: 1024 }
  fixedEdge.value = value
  fixedValue.value = value === 'width' ? current.width : current.height
  dependentValue.value = value === 'width' ? current.height : current.width
}

function applyCustomSize() {
  if (!customSize.value) return
  const value = `${customSize.value.width}x${customSize.value.height}`
  emit('update:size', value)
  const ratio = imageAspectRatio(customSize.value)
  emit('update:aspectRatio', schemaAspectRatios.value.includes(ratio) ? ratio : 'auto')
  customOpen.value = false
}
</script>

<template>
  <label>
    宽高比
    <n-select
      class="aspect-ratio-select"
      :value="effectiveAspectRatio"
      :options="aspectRatioOptions"
      @update:value="changeAspectRatio"
    />
  </label>
  <label>
    目标分辨率
    <n-select
      class="target-resolution-select"
      :value="size"
      :options="resolutionOptions"
      filterable
      @update:value="changeResolution"
    />
    <small class="parameter-hint">宽高比与分辨率保持一致；自定义尺寸仅提供模型支持的组合。若上游像素不符，系统仍会居中重采样到目标尺寸。</small>
  </label>
  <div v-if="customOpen" class="custom-size-editor">
    <div class="custom-size-heading">
      <strong>自定义尺寸</strong>
      <button type="button" aria-label="关闭自定义尺寸" @click="customOpen = false">×</button>
    </div>
    <n-radio-group :value="fixedEdge" size="small" @update:value="changeFixedEdge">
      <n-radio-button value="width">固定宽度</n-radio-button>
      <n-radio-button value="height">固定高度</n-radio-button>
    </n-radio-group>
    <div class="custom-size-fields">
      <label>
        {{ fixedEdge === 'width' ? '宽度' : '高度' }}
        <n-input-number
          v-model:value="fixedValue"
          :min="edgeMultiple"
          :max="maxEdge"
          :step="edgeMultiple"
          :precision="0"
        />
      </label>
      <label>
        {{ fixedEdge === 'width' ? '高度' : '宽度' }}
        <n-select
          v-model:value="dependentValue"
          :options="dependentSelectOptions"
          :disabled="!dependentSelectOptions.length"
          filterable
          placeholder="无可用尺寸"
        />
      </label>
    </div>
    <small v-if="!dependentSelectOptions.length" class="custom-size-error">
      当前数值不符合模型的边长、像素数或 {{ edgeMultiple }} 倍数限制。
    </small>
    <n-button type="primary" size="small" :disabled="!customSize" @click="applyCustomSize">
      使用 {{ customSize ? `${customSize.width} × ${customSize.height}` : '自定义尺寸' }}
    </n-button>
  </div>
</template>
