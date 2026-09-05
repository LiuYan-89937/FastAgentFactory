<template>
  <div class="tool-trace-message" :class="{ embedded }">
    <div v-if="!embedded" class="assistant-avatar" aria-hidden="true">
      <ComboFrameAnimation character="companion" action="idle" :size="34" paused />
    </div>
    <div class="trace-content">
      <div v-if="!embedded" class="trace-header">
        <strong>Combo</strong>
        <span>{{ formattedTime }}</span>
      </div>
      <details class="trace-group" open>
        <summary class="trace-caption">
          <span class="trace-caption-copy">
            <span>{{ t('tool.traceCount', { count: executions.length }) }}</span>
          </span>
          <span class="trace-chevron" aria-hidden="true">⌄</span>
        </summary>
        <ToolExecutionChain
          :executions="props.executions"
          :workspace-context="workspaceContext"
        />
      </details>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import ToolExecutionChain from '@/components/chat/ToolExecutionChain.vue'
import ComboFrameAnimation from '@/components/brand/ComboFrameAnimation.vue'
import { useI18n } from '@/composables/useI18n'
import type { ToolExecutionMessagePart } from '@/types/protocol'
import type { WorkspaceRequestContext } from '@/api/resourceTypes'

const props = withDefaults(defineProps<{
  executions: ToolExecutionMessagePart[]
  timestamp?: string
  workspaceContext?: WorkspaceRequestContext | null
  embedded?: boolean
}>(), {
  timestamp: '',
  workspaceContext: null,
  embedded: false,
})

const { locale, t } = useI18n()
const formattedTime = computed(() => new Date(props.timestamp || Date.now()).toLocaleTimeString(locale.value, {
  hour: '2-digit',
  minute: '2-digit',
}))

</script>

<style scoped>
.tool-trace-message {
  display: flex;
  gap: var(--app-space-md);
  padding: 8px var(--app-space-md);
}

.tool-trace-message.embedded {
  padding: 0 0 3px;
}

.assistant-avatar {
  display: grid;
  width: 40px;
  height: 36px;
  flex: 0 0 40px;
  place-items: center;
}

.trace-content {
  min-width: 0;
  flex: 1;
}

.trace-header {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin: 0 0 5px;
  font-size: 13px;
}

.trace-header strong {
  font-family: 'Avenir Next', 'SF Pro Display', 'Arial Rounded MT Bold', sans-serif;
  font-size: 16px;
  font-weight: 780;
  letter-spacing: -.055em;
}

.trace-header span {
  color: var(--app-text-muted);
  font-size: 11px;
}

.trace-caption {
  display: flex;
  width: fit-content;
  align-items: center;
  gap: 7px;
  margin-bottom: 2px;
  padding: 2px 0;
  color: var(--app-text-muted);
  font-size: 11px;
  cursor: pointer;
  list-style: none;
}

.trace-caption::-webkit-details-marker { display: none; }

.trace-caption-copy {
  display: flex;
  align-items: baseline;
  gap: 7px;
}

.trace-chevron {
  flex: 0 0 auto;
  transition: transform 160ms ease;
}

.trace-group[open] .trace-chevron {
  transform: rotate(180deg);
}

</style>
