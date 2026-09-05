<template>
  <div ref="panelRef" class="workspace-sidebar-content">
    <div v-if="sourceControlAvailable" class="workspace-source-pane" :style="sourcePaneStyle">
      <WorkspaceSourceControl
        :workspace-id="runtimeStore.activeWorkspaceId"
        :workspace-root="activeWorkspaceRoot"
        :active="uiStore.conversationDockPanel === 'workspace'"
      />
    </div>
    <div
      v-if="sourceControlAvailable"
      class="workspace-splitter"
      :class="{ dragging: resizing }"
      role="separator"
      aria-orientation="horizontal"
      :aria-label="t('workspace.resizeSections')"
      :aria-valuenow="Math.round(sourceRatio * 100)"
      tabindex="0"
      @pointerdown="startResize"
      @keydown="resizeWithKeyboard"
    >
      <span aria-hidden="true"></span>
    </div>
    <div class="workspace-content-pane">
      <div v-show="!previewLoading && !runtimeStore.workspaceFile" class="workspace-browser">
        <WorkspaceExplorer
          v-if="workspaceAvailable"
          class="workspace-sidebar-explorer"
          :workspace-context="workspaceRequestContext"
          @select-file="handleWorkspaceFileSelect"
        />
        <div v-else class="workspace-unavailable">
          <n-empty :description="t('workspace.noActiveSession')" size="small">
            <template #icon><ComboPngIcon name="empty-workspace" :size="60" /></template>
          </n-empty>
        </div>
      </div>
      <div v-if="previewLoading && !runtimeStore.workspaceFile" class="workspace-loading">
        <n-spin size="small" />
        <n-text depth="3">{{ t('workspace.readingFile') }}</n-text>
      </div>
      <FilePreview
        v-if="runtimeStore.workspaceFile"
        :file="runtimeStore.workspaceFile"
        :workspace-context="workspaceRequestContext"
        @close="closeWorkspacePreview"
        @deleted="closeWorkspacePreview"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch, type CSSProperties } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { NEmpty, NSpin, NText } from 'naive-ui'
import { useCommand } from '@/composables/useCommand'
import { useResourceContext } from '@/composables/useResourceContext'
import { useRuntimeStore } from '@/stores/runtime'
import { useAgentStore } from '@/stores/agent'
import { useUiStore } from '@/stores/ui'
import { useWorkspaceStore } from '@/stores/workspace'
import type { WorkspaceEntry } from '@/types/protocol'
import FilePreview from '@/components/workspace/FilePreview.vue'
import WorkspaceExplorer from '@/components/workspace/WorkspaceExplorer.vue'
import WorkspaceSourceControl from '@/components/workspace/WorkspaceSourceControl.vue'
import ComboPngIcon from '@/components/icons/ComboPngIcon.vue'
import { useI18n } from '@/composables/useI18n'

const uiStore = useUiStore()
const runtimeStore = useRuntimeStore()
const agentStore = useAgentStore()
const workspaceStore = useWorkspaceStore()
const commands = useCommand()
const resourceContext = useResourceContext()
const { t } = useI18n()
const previewLoading = ref(false)
const panelRef = ref<HTMLElement | null>(null)
const resizing = ref(false)
const resizePointerId = ref<number | null>(null)
const SOURCE_RATIO_STORAGE_KEY = 'combo.workspaceSourceControlRatio'
const SOURCE_RATIO_DEFAULT = 0.42
const SOURCE_RATIO_MIN = 0.22
const SOURCE_RATIO_MAX = 0.72
const WORKSPACE_PREVIEW_MAX_CHARS = 1_000_000
const sourceRatio = ref(loadSourceRatio())

const workspaceRequestContext = computed(() => resourceContext.workspaceContext.value)
const workspaceAvailable = computed(() => resourceContext.workspaceAvailable.value)
const activeWorkspaceRoot = computed(() => {
  const sessionId = runtimeStore.activeAgentSessionId || agentStore.selectedSessionId
  if (!sessionId) return ''
  const session = [...agentStore.agentSessions, ...agentStore.recentAgentSessions]
    .find(item => item.session_id === sessionId)
  return session?.workspace?.workdir_root || ''
})
const sourceControlAvailable = computed(() => (
  isTauri() && Boolean(activeWorkspaceRoot.value || runtimeStore.activeWorkspaceId)
))
const sourcePaneStyle = computed<CSSProperties>(() => ({
  flexBasis: `${sourceRatio.value * 100}%`,
}))

async function handleWorkspaceFileSelect(entry: WorkspaceEntry) {
  uiStore.setConversationDockPanel('workspace')
  previewLoading.value = true
  runtimeStore.workspaceFile = null
  await commands.readFile(workspaceStore.currentScope, entry.path, workspaceRequestContext.value, WORKSPACE_PREVIEW_MAX_CHARS)
  if (!runtimeStore.workspaceFile) {
    previewLoading.value = false
  }
}

function closeWorkspacePreview() {
  previewLoading.value = false
  runtimeStore.workspaceFile = null
}

function loadSourceRatio(): number {
  if (typeof window === 'undefined') return SOURCE_RATIO_DEFAULT
  try {
    const stored = Number(window.localStorage.getItem(SOURCE_RATIO_STORAGE_KEY))
    return Number.isFinite(stored) ? clampSourceRatio(stored) : SOURCE_RATIO_DEFAULT
  } catch {
    return SOURCE_RATIO_DEFAULT
  }
}

function saveSourceRatio() {
  try {
    window.localStorage.setItem(SOURCE_RATIO_STORAGE_KEY, String(sourceRatio.value))
  } catch {
    // The splitter remains usable for this session when persistent storage is unavailable.
  }
}

function startResize(event: PointerEvent) {
  if (event.button !== 0 || !panelRef.value) return
  event.preventDefault()
  resizing.value = true
  resizePointerId.value = event.pointerId
  window.addEventListener('pointermove', handleResize)
  window.addEventListener('pointerup', finishResize)
  window.addEventListener('pointercancel', finishResize)
}

function handleResize(event: PointerEvent) {
  if (event.pointerId !== resizePointerId.value) return
  const bounds = panelRef.value?.getBoundingClientRect()
  if (!bounds || bounds.height <= 0) return
  sourceRatio.value = clampSourceRatio((event.clientY - bounds.top) / bounds.height)
}

function finishResize(event: PointerEvent) {
  if (!resizing.value || event.pointerId !== resizePointerId.value) return
  resizing.value = false
  resizePointerId.value = null
  stopResizeListeners()
  saveSourceRatio()
}

function resizeWithKeyboard(event: KeyboardEvent) {
  if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return
  event.preventDefault()
  const direction = event.key === 'ArrowUp' ? -1 : 1
  sourceRatio.value = clampSourceRatio(sourceRatio.value + direction * 0.04)
  saveSourceRatio()
}

function clampSourceRatio(value: number): number {
  return Math.max(SOURCE_RATIO_MIN, Math.min(SOURCE_RATIO_MAX, value))
}

function stopResizeListeners() {
  window.removeEventListener('pointermove', handleResize)
  window.removeEventListener('pointerup', finishResize)
  window.removeEventListener('pointercancel', finishResize)
}

watch(
  () => runtimeStore.workspaceFile,
  (file) => {
    if (file) previewLoading.value = false
  }
)

watch(
  () => resourceContext.workspaceContextKey.value,
  () => {
    workspaceStore.setScope(resourceContext.workspaceDefaultScope.value)
    closeWorkspacePreview()
  },
  { immediate: true }
)

onBeforeUnmount(stopResizeListeners)
</script>

<style scoped>
.workspace-sidebar-content {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.workspace-source-pane,
.workspace-content-pane {
  min-height: 0;
  overflow: hidden;
}

.workspace-source-pane {
  flex: 0 0 auto;
}

.workspace-source-pane :deep(.source-control) {
  height: 100%;
  min-height: 0;
  overflow-y: auto;
  border-bottom: 0;
}

.workspace-content-pane {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
}

.workspace-splitter {
  position: relative;
  z-index: 2;
  height: 9px;
  flex: 0 0 9px;
  display: grid;
  place-items: center;
  cursor: row-resize;
  touch-action: none;
  background: var(--app-surface);
}

.workspace-splitter::before {
  position: absolute;
  inset: 0;
  border-top: 1px solid var(--app-divider);
  border-bottom: 1px solid var(--app-divider);
  content: '';
  opacity: 0;
  transition: opacity var(--app-transition-fast);
}

.workspace-splitter span {
  position: relative;
  width: 34px;
  height: 3px;
  border-radius: var(--app-radius-pill);
  background: var(--app-border-hover);
  transition: width var(--app-transition-fast), background-color var(--app-transition-fast);
}

.workspace-splitter:hover::before,
.workspace-splitter:focus-visible::before,
.workspace-splitter.dragging::before {
  opacity: 1;
}

.workspace-splitter:hover span,
.workspace-splitter:focus-visible span,
.workspace-splitter.dragging span {
  width: 48px;
  background: var(--app-text-secondary);
}

.workspace-browser {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.workspace-sidebar-explorer {
  flex: 1;
  min-height: 0;
}

.workspace-unavailable {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--app-space-xl);
}

.workspace-loading {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--app-space-sm);
  color: var(--app-text-muted);
}
</style>
