export type ThemePreference = 'light' | 'dark' | 'system'

export interface CurrentUser {
  id: string
  username: string
  displayName: string | null
  role: 'admin' | 'user'
  mustChangePassword: boolean
  themePreference: ThemePreference
}

export interface Provider {
  id: string
  providerKey: string
  providerType: string
  displayName: string
  baseUrl: string
  enabled: boolean
  configJson: Record<string, unknown>
  credentialConfigured: boolean
  modelCount: number
  healthStatus: 'unknown' | 'healthy' | 'unhealthy'
  lastHealthCheckedAt: string | null
  lastHealthError: string | null
  createdAt: string
  updatedAt: string
}

export interface ParameterDefinition {
  type: 'boolean' | 'integer' | 'number' | 'enum' | 'string'
  default?: unknown
  min?: number
  max?: number
  step?: number
  options?: string[]
  allow_custom?: boolean
  supported?: boolean
  operations?: Array<'generation' | 'edit'>
  visible_when?: Record<string, unknown | unknown[]>
}

export interface ImageModel {
  id: string
  providerId: string
  providerType: string
  modelKey: string
  upstreamModelId: string
  displayName: string
  capabilities: Record<string, unknown>
  parameterSchema: {
    meta?: Record<string, unknown>
    parameters?: Record<string, ParameterDefinition>
  }
  availabilityStatus: 'discovered' | 'verified' | 'unsupported' | 'unavailable'
  discoverySource: string
  capabilitySource: string
  lastDiscoveredAt: string | null
  lastVerifiedAt: string | null
  enabled: boolean
}

export interface ImageAsset {
  id: string
  contentUrl: string
  mimeType: string
  width: number | null
  height: number | null
  fileSizeBytes: number
}

export interface MessageImageAsset extends ImageAsset {
  relationType: 'attachment' | 'reference' | 'generated'
}

export interface Conversation {
  id: string
  title: string
  status: 'active' | 'archived'
  defaultProviderId: string | null
  defaultModelId: string | null
  sortOrder: number
  lastMessageAt: string
  createdAt: string
  updatedAt: string
}

export interface ConversationMessage {
  id: string
  conversationId: string
  parentMessageId: string | null
  role: 'system' | 'user' | 'assistant'
  status: 'pending' | 'streaming' | 'completed' | 'failed' | 'cancelled'
  sequenceNo: number
  content: string | null
  metadata: Record<string, unknown>
  taskId: string | null
  taskErrorCode: string | null
  taskErrorMessage: string | null
  taskRetryCount: number | null
  taskStartedAt: string | null
  taskFinishedAt: string | null
  assets: MessageImageAsset[]
  createdAt: string
  updatedAt: string
}

export interface ConversationDetail extends Conversation {
  messages: ConversationMessage[]
}

export interface PromptTemplate {
  id: string
  ownerId: string | null
  templateType: 'general' | 'style'
  title: string
  prompt: string
  negativePrompt: string | null
  tags: string[]
  isPublic: boolean
  enabled: boolean
}

export interface TaskEvent {
  id: string
  type: string
  data: Record<string, unknown>
}
