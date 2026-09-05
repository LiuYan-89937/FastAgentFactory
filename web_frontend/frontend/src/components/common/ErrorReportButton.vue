<template>
  <n-button
    v-if="available"
    :size="size"
    :secondary="secondary"
    :quaternary="quaternary"
    :type="type"
    @click="open = true"
  >
    <template #icon><n-icon><BugOutline /></n-icon></template>
    {{ t('errorReport.action') }}
  </n-button>

  <n-modal
    v-if="available"
    v-model:show="open"
    preset="card"
    class="error-report-dialog"
    :title="t('errorReport.title')"
    style="width: min(520px, calc(100vw - 32px)); max-width: 520px"
  >
    <div class="error-report-content">
      <p>{{ t('errorReport.description') }}</p>
      <n-input
        v-model:value="note"
        type="textarea"
        :autosize="{ minRows: 3, maxRows: 7 }"
        :placeholder="t('errorReport.notePlaceholder')"
      />
      <small>{{ t('errorReport.privacy') }}</small>
      <div class="error-report-actions">
        <n-button @click="open = false">{{ t('common.cancel') }}</n-button>
        <n-button
          type="primary"
          :loading="submitting"
          :disabled="!canSubmit"
          @click="submit"
        >
          {{ t('errorReport.submit') }}
        </n-button>
      </div>
    </div>
  </n-modal>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { NButton, NIcon, NInput, NModal, useMessage } from 'naive-ui'
import { BugOutline } from '@/components/icons'
import { errorReportsApi } from '@/api/errorReports'
import { useI18n } from '@/composables/useI18n'

const props = withDefaults(defineProps<{
  summary?: string
  errorCode?: string
  requestId?: string
  diagnosticRef?: string
  context?: Record<string, unknown>
  size?: 'tiny' | 'small' | 'medium' | 'large'
  type?: 'default' | 'primary' | 'info' | 'success' | 'warning' | 'error'
  secondary?: boolean
  quaternary?: boolean
}>(), {
  summary: '',
  errorCode: '',
  requestId: '',
  diagnosticRef: '',
  context: () => ({}),
  size: 'small',
  type: 'default',
  secondary: true,
  quaternary: false,
})

const { t } = useI18n()
const message = useMessage()
const available = errorReportsApi.available()
const open = ref(false)
const note = ref('')
const submitting = ref(false)
const canSubmit = computed(() => Boolean(props.summary.trim() || note.value.trim()))

async function submit(): Promise<void> {
  if (!canSubmit.value || submitting.value) return
  submitting.value = true
  try {
    const summary = props.summary.trim() || note.value.trim()
    const context = note.value.trim()
      ? { ...props.context, user_note: note.value.trim() }
      : props.context
    const receipt = await errorReportsApi.submit({
      summary,
      errorCode: props.errorCode,
      requestId: props.requestId,
      diagnosticRef: props.diagnosticRef,
      context,
    })
    message.success(t('errorReport.succeeded', { id: receipt.error_report_id.slice(0, 8) }))
    note.value = ''
    open.value = false
  } catch (error) {
    message.error(error instanceof Error ? error.message : t('errorReport.failed'))
  } finally {
    submitting.value = false
  }
}
</script>

<style scoped>
.error-report-content {
  display: grid;
  gap: var(--app-space-md);
}

.error-report-content p,
.error-report-content small {
  color: var(--app-text-muted);
  line-height: var(--app-leading-normal);
}

.error-report-content small {
  font-size: var(--app-font-xs);
}

.error-report-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--app-space-sm);
}
</style>
