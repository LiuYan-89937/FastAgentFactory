export const COMPUTER_USE_TOOL_ID = 'computer_use'

export function isComputerUseToolName(value: unknown): boolean {
  const name = String(value || '').trim()
  if (!name) return false
  if (name === COMPUTER_USE_TOOL_ID) return true
  if (!name.startsWith('tool://builtin/')) return false
  return name.slice('tool://builtin/'.length).split('/')[0] === COMPUTER_USE_TOOL_ID
}
