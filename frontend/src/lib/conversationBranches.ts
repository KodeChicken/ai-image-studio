import type { ConversationMessage } from '@/types/api'

export interface BranchPosition {
  index: number
  total: number
  siblings: ConversationMessage[]
}

function bySequence(left: ConversationMessage, right: ConversationMessage) {
  return left.sequenceNo - right.sequenceNo
}

export function latestMessageId(messages: ConversationMessage[]) {
  return messages.reduce<ConversationMessage | null>(
    (latest, item) => (!latest || item.sequenceNo > latest.sequenceNo ? item : latest),
    null,
  )?.id ?? null
}

export function branchPath(messages: ConversationMessage[], leafId: string | null) {
  const byId = new Map(messages.map((item) => [item.id, item]))
  let current = (leafId ? byId.get(leafId) : undefined) ?? byId.get(latestMessageId(messages) ?? '')
  const visited = new Set<string>()
  const path: ConversationMessage[] = []

  while (current && !visited.has(current.id)) {
    path.push(current)
    visited.add(current.id)
    current = current.parentMessageId ? byId.get(current.parentMessageId) : undefined
  }

  return path.reverse()
}

export function branchPosition(messages: ConversationMessage[], message: ConversationMessage): BranchPosition {
  const siblings = messages
    .filter((item) => item.parentMessageId === message.parentMessageId && item.role === message.role)
    .sort(bySequence)
  return {
    index: siblings.findIndex((item) => item.id === message.id),
    total: siblings.length,
    siblings,
  }
}

export function latestDescendantId(messages: ConversationMessage[], rootId: string) {
  const root = messages.find((item) => item.id === rootId)
  if (!root) return null

  const children = new Map<string, ConversationMessage[]>()
  for (const item of messages) {
    if (!item.parentMessageId) continue
    const entries = children.get(item.parentMessageId) ?? []
    entries.push(item)
    children.set(item.parentMessageId, entries)
  }

  let latest = root
  const pending = [root]
  const visited = new Set<string>()
  while (pending.length) {
    const current = pending.pop()
    if (!current || visited.has(current.id)) continue
    visited.add(current.id)
    if (current.sequenceNo > latest.sequenceNo) latest = current
    pending.push(...(children.get(current.id) ?? []))
  }
  return latest.id
}
