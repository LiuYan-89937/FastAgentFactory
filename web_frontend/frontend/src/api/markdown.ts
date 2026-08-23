import { requestJson } from './http'

export interface MermaidRepairResult {
  source: string
}

export const markdownApi = {
  repairMermaid: (source: string, parserError: string) => requestJson<MermaidRepairResult>(
    '/api/runtime/markdown/mermaid/repair',
    {
      method: 'POST',
      body: JSON.stringify({ source, parser_error: parserError }),
    },
  ),
}
