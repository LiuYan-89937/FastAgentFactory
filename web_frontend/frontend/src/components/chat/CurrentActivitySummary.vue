<template>
  <div
    class="current-activity-summary"
    :class="[`activity-${activity.status}`, `activity-kind-${activity.kind}`]"
    role="status"
    aria-live="polite"
    :aria-label="activity.text"
  >
    <div class="current-activity-heading">
      <ComboFrameAnimation
        character="lead"
        action="running"
        :size="22"
        aria-hidden="true"
      />
      <span class="current-activity-text">{{ activity.text }}</span>
      <span v-if="elapsedText" class="current-activity-elapsed">{{ elapsedText }}</span>
    </div>

    <div v-if="activity.kind === 'computer_use'" class="computer-use-observation">
      <section class="computer-frame-panel">
        <header class="observation-header">
          <span>{{ t('conversation.computerUse.currentFrame') }}</span>
          <span v-if="activity.frame">#{{ activity.frame.frameId }}</span>
        </header>
        <div class="computer-frame-stage">
          <img
            v-if="frameSource"
            :src="frameSource"
            :alt="t('conversation.computerUse.frameAlt', { frame: activity.frame?.frameId || 0 })"
          />
          <span v-else>{{ t('conversation.computerUse.waitingFrame') }}</span>
        </div>
      </section>

      <section class="accessibility-panel">
        <header class="observation-header">
          <span>AX Tree</span>
          <span>{{ accessibilityMeta }}</span>
        </header>
        <div class="accessibility-tree">
          <div
            v-for="row in accessibilityRows"
            :key="row.key"
            class="accessibility-node"
            :style="{ paddingLeft: row.indent }"
          >
            <span class="accessibility-role">{{ row.role }}</span>
            <span v-if="row.name" class="accessibility-name">{{ row.name }}</span>
            <span v-if="row.value" class="accessibility-value">{{ row.value }}</span>
          </div>
          <span v-if="accessibilityState" class="observation-empty">{{ accessibilityState }}</span>
        </div>
      </section>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import ComboFrameAnimation from '@/components/brand/ComboFrameAnimation.vue'
import type { ConversationActivitySummary } from '@/composables/conversation/useConversationMessageProjection'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{
  activity: ConversationActivitySummary
}>()
const { t } = useI18n()

const now = ref(Date.now())
let elapsedTimer: number | undefined
const elapsedText = computed(() => {
  if (props.activity.kind !== 'computer_use' || !props.activity.startedAt) return ''
  const startedAt = Date.parse(props.activity.startedAt)
  if (!Number.isFinite(startedAt)) return ''
  const elapsedSeconds = Math.max(0, Math.floor((now.value - startedAt) / 1000))
  if (elapsedSeconds < 60) return `${elapsedSeconds}s`
  return `${Math.floor(elapsedSeconds / 60)}m ${elapsedSeconds % 60}s`
})
const frameSource = computed(() => {
  const frame = props.activity.frame
  return frame ? `data:${frame.mimeType};base64,${frame.data}` : ''
})
const accessibilityMeta = computed(() => {
  const accessibility = props.activity.accessibility
  if (!accessibility) {
    const target = props.activity.target
    return [target?.displayName, target?.windowTitle].filter(Boolean).join(' · ')
      || t('conversation.computerUse.waitingAxTree')
  }
  const location = [accessibility.application, accessibility.windowTitle].filter(Boolean).join(' · ')
  const count = t('conversation.computerUse.nodeCount', { count: accessibility.nodes.length })
  return [location, count].filter(Boolean).join(' · ')
})
const accessibilityState = computed(() => {
  const accessibility = props.activity.accessibility
  if (!accessibility) return t('conversation.computerUse.waitingAxTree')
  if (!accessibility.available) {
    return accessibility.error || t('conversation.computerUse.axTreeUnavailable')
  }
  return accessibility.nodes.length === 0 ? t('conversation.computerUse.axTreeEmpty') : ''
})
const accessibilityRows = computed(() => {
  const nodes = props.activity.accessibility?.nodes || []
  const parents = new Map<number, number | null>()
  nodes.forEach((node) => {
    const id = numberValue(node.element_id)
    if (id !== null) parents.set(id, numberValue(node.parent_id))
  })
  return nodes.map((node, index) => {
    const id = numberValue(node.element_id)
    return {
      key: id === null ? `node-${index}` : `node-${id}`,
      indent: `${5 + Math.min(nodeDepth(id, parents), 8) * 10}px`,
      role: String(node.role || 'element').replace(/^AX/, ''),
      name: compactText(node.name),
      value: compactText(node.value),
    }
  })
})

onMounted(() => {
  elapsedTimer = window.setInterval(() => {
    now.value = Date.now()
  }, 1000)
})

onBeforeUnmount(() => {
  if (elapsedTimer !== undefined) window.clearInterval(elapsedTimer)
})

function numberValue(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function nodeDepth(id: number | null, parents: Map<number, number | null>): number {
  if (id === null) return 0
  const visited = new Set<number>()
  let current = parents.get(id) ?? null
  let depth = 0
  while (current !== null && !visited.has(current) && depth < 12) {
    visited.add(current)
    depth += 1
    current = parents.get(current) ?? null
  }
  return depth
}

function compactText(value: unknown): string {
  return String(value || '').replace(/\s+/g, ' ').trim()
}
</script>

<style scoped>
.current-activity-summary {
  display: inline-block;
  max-width: min(72vw, 720px);
  min-height: 22px;
  padding: 2px 8px 3px 50px;
  color: var(--app-text-tertiary);
  font-size: 12px;
  line-height: 18px;
}

.current-activity-heading {
  display: flex;
  align-items: flex-start;
  gap: 7px;
}

.current-activity-text {
  min-width: 0;
  white-space: normal;
  overflow-wrap: anywhere;
}

.activity-kind-computer_use {
  width: min(820px, calc(100% - 58px));
  max-width: none;
  color: var(--app-primary-color);
}

.current-activity-elapsed {
  flex: none;
  color: var(--app-text-tertiary);
  font-variant-numeric: tabular-nums;
}

.computer-use-observation {
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(260px, .85fr);
  height: 270px;
  margin-top: 9px;
  overflow: hidden;
  border: 1px solid var(--app-border);
  border-radius: 16px;
  background: var(--app-surface);
  box-shadow: var(--app-shadow-sm);
  color: var(--app-text);
}

.computer-frame-panel,
.accessibility-panel {
  min-width: 0;
  min-height: 0;
  display: grid;
  grid-template-rows: 34px minmax(0, 1fr);
}

.accessibility-panel {
  border-left: 1px solid var(--app-border);
}

.observation-header {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 0 11px;
  border-bottom: 1px solid var(--app-border);
  background: var(--app-surface-muted);
  color: var(--app-text-secondary);
  font-size: 10px;
}

.observation-header span:last-child {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.computer-frame-stage {
  min-height: 0;
  display: grid;
  place-items: center;
  overflow: hidden;
  background: #111;
  color: rgba(255, 255, 255, .56);
  font-size: 11px;
}

.computer-frame-stage img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.accessibility-tree {
  min-height: 0;
  overflow: auto;
  overscroll-behavior: contain;
  padding: 6px;
  font: 10px/1.55 var(--app-font-mono);
}

.accessibility-node {
  min-width: max-content;
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 3px 5px;
  border-radius: 6px;
}

.accessibility-node:hover {
  background: var(--app-surface-hover);
}

.accessibility-role {
  color: var(--app-primary-color);
}

.accessibility-name {
  color: var(--app-text);
}

.accessibility-value {
  max-width: 260px;
  overflow: hidden;
  color: var(--app-text-muted);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.observation-empty {
  display: grid;
  min-height: 100%;
  place-items: center;
  padding: 16px;
  color: var(--app-text-muted);
  text-align: center;
}

@media (max-width: 760px) {
  .computer-use-observation {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(0, 1fr) minmax(110px, .7fr);
    height: 420px;
  }

  .accessibility-panel {
    border-top: 1px solid var(--app-border);
    border-left: 0;
  }
}
</style>
