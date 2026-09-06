<template>
  <div v-if="!cancelled" class="runtime-error-card" role="alert">
    <div class="runtime-error-summary">
      <n-icon class="runtime-error-icon" :size="18">
        <AlertCircleOutline />
      </n-icon>
      <div class="runtime-error-copy">
        <strong>{{ t('chat.runtimeErrorTitle') }}</strong>
        <span>{{ part.message }}</span>
      </div>
    </div>

    <div class="runtime-error-report-action">
      <ErrorReportButton
        :summary="part.message"
        :error-code="errorCode"
        :request-id="requestId"
        :diagnostic-ref="diagnosticRef"
        :context="reportContext"
        size="tiny"
        type="error"
        quaternary
        :secondary="false"
      />
    </div>

    <details v-if="formattedDetails" class="runtime-error-details">
      <summary>{{ t('common.details') }}</summary>
      <pre>{{ formattedDetails }}</pre>
    </details>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { NIcon } from 'naive-ui'
import { AlertCircleOutline } from '@/components/icons'
import ErrorReportButton from '@/components/common/ErrorReportButton.vue'
import { useI18n } from '@/composables/useI18n'
import type { ErrorMessagePart } from '@/types/protocol'
import { isRuntimeCancellation } from '@/utils/runtimeCancellation'

const props = defineProps<{
  part: ErrorMessagePart
}>()

const { t } = useI18n()
const errorEnvelope = computed<Record<string, unknown>>(() => (
  isRecord(props.part.details) ? props.part.details : {}
))
const cancelled = computed(() => isRuntimeCancellation(errorEnvelope.value))
const errorCode = computed(() => stringValue(errorEnvelope.value.code))
const requestId = computed(() => stringValue(errorEnvelope.value.request_id))
const diagnosticRef = computed(() => stringValue(errorEnvelope.value.diagnostic_ref))
const reportContext = computed<Record<string, unknown>>(() => ({
  category: stringValue(errorEnvelope.value.category),
  operation: stringValue(errorEnvelope.value.operation),
  runtime_instance_id: stringValue(errorEnvelope.value.runtime_instance_id),
  details: isRecord(errorEnvelope.value.details) ? errorEnvelope.value.details : {},
}))
const formattedDetails = computed(() => {
  if (Object.keys(errorEnvelope.value).length === 0) return ''
  return JSON.stringify(errorEnvelope.value, null, 2)
})

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

function stringValue(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}
</script>

<style scoped>
.runtime-error-card {
  width: min(680px, 100%);
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--app-error) 30%, var(--app-border));
  border-radius: var(--app-radius-lg);
  background: color-mix(in srgb, var(--app-error) 5%, var(--app-surface));
}

.runtime-error-summary {
  display: flex;
  align-items: center;
  gap: var(--app-space-sm);
  padding: 9px 11px;
}

.runtime-error-icon {
  flex: 0 0 auto;
  color: var(--app-error);
}

.runtime-error-copy {
  display: grid;
  flex: 1;
  min-width: 0;
  gap: 1px;
}

.runtime-error-copy strong {
  color: var(--app-text);
  font-size: 13px;
}

.runtime-error-copy span {
  overflow-wrap: anywhere;
  color: var(--app-text-muted);
  font-size: 12px;
  line-height: 1.45;
}

.runtime-error-report-action {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 11px 9px;
}

.runtime-error-details {
  border-top: 1px solid var(--app-border);
}

.runtime-error-details summary {
  padding: 7px 11px;
  color: var(--app-text-muted);
  cursor: pointer;
  font-size: 12px;
}

.runtime-error-details pre {
  max-height: 220px;
  margin: 0;
  overflow: auto;
  padding: 0 11px 11px;
  color: var(--app-text-muted);
  font: 11px/1.5 var(--app-font-mono);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
