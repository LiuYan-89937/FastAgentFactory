import type {
  ComputerUseActivityView,
  RuntimeFrontendEvent,
  RuntimeViewState,
} from '@/types/protocol'
import { isComputerUseToolName } from '@/utils/computerUse'
import { toolPayloadValue } from './toolPayload'

type ComputerUseMutationState = Pick<RuntimeViewState, 'computerUseActivity'>

export function applyComputerUseLifecycleEvent(
  state: ComputerUseMutationState,
  event: RuntimeFrontendEvent,
  status: ComputerUseActivityView['status'],
): boolean {
  if (!eventBelongsToComputerUse(state, event)) return false
  const payload = event.payload || {}
  const progress = objectValue(payload.output)
  const current = state.computerUseActivity
  const toolCallId = toolPayloadValue(payload, ['tool_call_id', 'toolCallId'])
  const retainsObservation = isSameComputerUseActivity(
    current,
    event.request_id || null,
    toolCallId ? String(toolCallId) : null,
  )
  const nextStatus = terminalComputerUseStatus(current.status, status, event.event_type)
  state.computerUseActivity = {
    status: nextStatus,
    requestId: event.request_id || current.requestId || null,
    toolCallId: toolCallId ? String(toolCallId) : current.toolCallId || null,
    phase: phaseForEvent(event, progress, nextStatus),
    step: optionalNumber(progress?.step) ?? (retainsObservation ? current.step : null) ?? null,
    actionCount: optionalNumber(progress?.action_count)
      ?? (retainsObservation ? current.actionCount : null)
      ?? null,
    message: optionalText(progress?.message)
      || optionalText(payload.message)
      || (retainsObservation ? current.message : null)
      || null,
    startedAt: startedAtForEvent(current, event, retainsObservation),
    updatedAt: event.timestamp,
    frame: frameView(progress?.frame) || (retainsObservation ? current.frame : null) || null,
    target: targetView(progress?.target) || (retainsObservation ? current.target : null) || null,
    accessibility: accessibilityView(progress?.accessibility)
      || (retainsObservation ? current.accessibility : null)
      || null,
  }
  return true
}

function isSameComputerUseActivity(
  current: ComputerUseActivityView,
  requestId: string | null,
  toolCallId: string | null,
): boolean {
  if (!requestId || current.requestId !== requestId) return false
  if (!toolCallId || !current.toolCallId) return true
  return current.toolCallId === toolCallId
}

export function applyComputerUseApprovalRequest(
  state: ComputerUseMutationState,
  event: RuntimeFrontendEvent,
  request: Record<string, any>,
): boolean {
  if (!isComputerUsePayload(request)) return false
  return applyComputerUseLifecycleEvent(
    state,
    { ...event, payload: { ...(event.payload || {}), ...request } },
    'approval',
  )
}

export function resolveComputerUseApproval(
  state: ComputerUseMutationState,
  event: RuntimeFrontendEvent,
  toolCallIds: string[],
): void {
  const current = state.computerUseActivity
  if (current.status !== 'approval') return
  if (toolCallIds.length > 0 && current.toolCallId && !toolCallIds.includes(current.toolCallId)) return
  const approved = Boolean(event.payload?.approved)
  state.computerUseActivity = {
    ...current,
    status: approved ? 'running' : 'cancelled',
    phase: approved ? 'starting' : 'cancelled',
    updatedAt: event.timestamp,
  }
}

export function finalizeComputerUseForRequest(
  state: ComputerUseMutationState,
  requestId: string | null,
  timestamp: string,
  status: 'cancelled' | 'failed',
  message?: string,
): void {
  const current = state.computerUseActivity
  if (!requestId || current.requestId !== requestId) return
  if (current.status !== 'running' && current.status !== 'approval') return
  state.computerUseActivity = {
    ...current,
    status,
    phase: status,
    message: optionalText(message) || current.message || null,
    updatedAt: timestamp,
  }
}

export function clearComputerUseForRequest(
  state: ComputerUseMutationState,
  requestId: string | null,
): void {
  const current = state.computerUseActivity
  if (!requestId || current.requestId !== requestId) return
  state.computerUseActivity = { status: 'idle' }
}

function eventBelongsToComputerUse(
  state: ComputerUseMutationState,
  event: RuntimeFrontendEvent,
): boolean {
  if (isComputerUsePayload(event.payload || {})) return true
  const toolCallId = toolPayloadValue(event.payload || {}, ['tool_call_id', 'toolCallId'])
  return Boolean(
    toolCallId
    && state.computerUseActivity.toolCallId
    && String(toolCallId) === state.computerUseActivity.toolCallId,
  )
}

function isComputerUsePayload(payload: Record<string, any>): boolean {
  return ['tool_name', 'tool_id', 'name']
    .some(key => isComputerUseToolName(payload[key]))
}

function terminalComputerUseStatus(
  current: ComputerUseActivityView['status'],
  incoming: ComputerUseActivityView['status'],
  eventType: string,
): ComputerUseActivityView['status'] {
  if (eventType === 'tool_observation_available' && ['completed', 'failed', 'cancelled'].includes(current)) {
    return current
  }
  return incoming
}

function phaseForEvent(
  event: RuntimeFrontendEvent,
  progress: Record<string, any> | null,
  status: ComputerUseActivityView['status'],
): string {
  const progressPhase = optionalText(progress?.phase)
  if (progressPhase) return progressPhase
  if (status === 'approval') return 'approval'
  if (status === 'completed' || status === 'failed' || status === 'cancelled') return status
  if (event.event_type === 'tool_call_proposed') return 'preparing'
  return 'starting'
}

function startedAtForEvent(
  current: ComputerUseActivityView,
  event: RuntimeFrontendEvent,
  retainsActivity: boolean,
): string | null {
  if (event.event_type === 'tool_call_started') return event.timestamp
  if (retainsActivity && current.startedAt) return current.startedAt
  if (event.event_type === 'tool_call_output_delta') return event.timestamp
  return null
}

function objectValue(value: unknown): Record<string, any> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, any>
    : null
}

function frameView(value: unknown) {
  const frame = objectValue(value)
  if (!frame) return null
  const frameId = optionalNumber(frame.frame_id)
  const width = optionalNumber(frame.width)
  const height = optionalNumber(frame.height)
  const mimeType = optionalText(frame.mime_type)
  const data = optionalText(frame.data)
  if (frameId === null || width === null || height === null || !mimeType || !data) return null
  return { frameId, width, height, mimeType, data }
}

function accessibilityView(value: unknown) {
  const accessibility = objectValue(value)
  if (!accessibility || !Array.isArray(accessibility.nodes)) return null
  return {
    available: accessibility.available === true,
    application: optionalText(accessibility.application) || '',
    windowTitle: optionalText(accessibility.window_title) || '',
    error: optionalText(accessibility.error),
    nodes: accessibility.nodes.filter(
      (node): node is Record<string, any> => Boolean(node && typeof node === 'object' && !Array.isArray(node)),
    ),
  }
}

function targetView(value: unknown) {
  const target = objectValue(value)
  if (!target) return null
  const applicationId = optionalText(target.application_id)
  const displayName = optionalText(target.display_name)
  const processId = optionalNumber(target.process_id)
  const windowId = optionalNumber(target.window_id)
  if (!applicationId || !displayName || processId === null || windowId === null) return null
  return {
    applicationId,
    displayName,
    processId,
    windowId,
    windowTitle: optionalText(target.window_title) || '',
  }
}

function optionalText(value: unknown): string | null {
  const text = String(value || '').trim()
  return text || null
}

function optionalNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}
