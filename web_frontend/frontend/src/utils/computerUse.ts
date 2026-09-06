import type { ChatMessagePart } from '@/types/protocol'

export const COMPUTER_USE_TOOL_ID = 'computer_use'

export function isComputerUseToolName(value: unknown): boolean {
  const name = String(value || '').trim()
  if (!name) return false
  if (name === COMPUTER_USE_TOOL_ID) return true
  if (!name.startsWith('tool://builtin/')) return false
  return name.slice('tool://builtin/'.length).split('/')[0] === COMPUTER_USE_TOOL_ID
}

export function isComputerUseToolActivity(activity: any): boolean {
  return isComputerUseToolName(activity.toolName)
    || isComputerUseToolName(activity.tool_name)
    || isComputerUseToolName(activity.tool_id)
    || isComputerUseToolName(activity.payload?.tool_id)
    || isComputerUseToolName(activity.payload?.tool_name)
}

export function withoutComputerUseParts(parts: ChatMessagePart[]): ChatMessagePart[] {
  return parts.filter((part) => {
    if (part.type !== 'tool_call' && part.type !== 'tool_result' && part.type !== 'tool_execution') {
      return true
    }
    return !isComputerUseToolName(part.toolName)
  })
}
