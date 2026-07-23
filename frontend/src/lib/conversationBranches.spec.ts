import { describe, expect, it } from 'vitest'
import type { ConversationMessage } from '@/types/api'
import { branchPath, branchPosition, latestDescendantId, latestMessageId } from './conversationBranches'

function message(
  id: string,
  parentMessageId: string | null,
  role: ConversationMessage['role'],
  sequenceNo: number,
): ConversationMessage {
  return {
    id,
    conversationId: 'conversation-1',
    parentMessageId,
    role,
    status: 'completed',
    sequenceNo,
    content: id,
    metadata: {},
    taskId: null,
    taskErrorCode: null,
    taskErrorMessage: null,
    taskRetryCount: null,
    taskStartedAt: null,
    taskFinishedAt: null,
    assets: [],
    createdAt: '2026-07-21T10:00:00Z',
    updatedAt: '2026-07-21T10:00:00Z',
  }
}

const branchedMessages = [
  message('user-1', null, 'user', 1),
  message('assistant-1', 'user-1', 'assistant', 2),
  message('user-2a', 'assistant-1', 'user', 3),
  message('assistant-2a', 'user-2a', 'assistant', 4),
  message('user-2b', 'assistant-1', 'user', 5),
  message('assistant-2b', 'user-2b', 'assistant', 6),
]

describe('conversation branch helpers', () => {
  it('shows only the path to the latest message by default', () => {
    expect(latestMessageId(branchedMessages)).toBe('assistant-2b')
    expect(branchPath(branchedMessages, null).map((item) => item.id)).toEqual([
      'user-1',
      'assistant-1',
      'user-2b',
      'assistant-2b',
    ])
  })

  it('reports sibling branch position and selects another branch descendant', () => {
    const branch = branchPosition(branchedMessages, branchedMessages[2]!)
    expect(branch.index).toBe(0)
    expect(branch.total).toBe(2)
    expect(latestDescendantId(branchedMessages, branch.siblings[1]!.id)).toBe('assistant-2b')
  })

  it('finds the most recently extended descendant instead of the newest direct child', () => {
    const messages = [
      ...branchedMessages,
      message('user-3a', 'assistant-2a', 'user', 7),
      message('assistant-3a', 'user-3a', 'assistant', 8),
    ]
    expect(latestDescendantId(messages, 'user-2a')).toBe('assistant-3a')
  })

  it('falls back to the latest valid path when a saved leaf no longer exists', () => {
    const path = branchPath(branchedMessages, 'missing')
    expect(path[path.length - 1]?.id).toBe('assistant-2b')
  })
})
