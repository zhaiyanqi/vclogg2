export interface SessionInfo {
  id: string
  username: string
  csrfToken: string
  roleIds: string[]
  permissions: string[]
  superAdmin: boolean
}

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code = 'request_failed'
  ) {
    super(message)
  }
}

let csrfToken = ''

export function setCSRF(value: string) {
  csrfToken = value
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers)
  if (init.body && !(init.body instanceof FormData)) headers.set('Content-Type', 'application/json')
  if (csrfToken && init.method && !['GET', 'HEAD'].includes(init.method)) headers.set('X-CSRF-Token', csrfToken)
  const response = await fetch(path, { ...init, headers, credentials: 'same-origin' })
  if (!response.ok) {
    let message = `请求失败 (${response.status})`
    let code = 'request_failed'
    try {
      const body = await response.json()
      message = body?.error?.message || message
      code = body?.error?.code || code
    } catch {
      // Preserve the status-based fallback for non-JSON failures.
    }
    throw new ApiError(message, response.status, code)
  }
  if (response.status === 204) return undefined as T
  const type = response.headers.get('content-type') || ''
  return (type.includes('json') ? response.json() : response) as Promise<T>
}

export function uploadForm<T>(path: string, body: FormData, onProgress?: (percent: number) => void): Promise<T> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest()
    request.open('POST', path)
    request.withCredentials = true
    if (csrfToken) request.setRequestHeader('X-CSRF-Token', csrfToken)
    request.upload.onprogress = (event) => {
      if (event.lengthComputable && event.total > 0) onProgress?.(Math.round(event.loaded * 100 / event.total))
    }
    request.onerror = () => reject(new ApiError('网络连接失败', 0, 'network_error'))
    request.onload = () => {
      let payload: any
      try {
        payload = request.responseText ? JSON.parse(request.responseText) : undefined
      } catch {
        payload = undefined
      }
      if (request.status < 200 || request.status >= 300) {
        reject(new ApiError(payload?.error?.message || `请求失败 (${request.status})`, request.status, payload?.error?.code))
        return
      }
      onProgress?.(100)
      resolve(payload as T)
    }
    request.send(body)
  })
}

export const jsonBody = (value: unknown): BodyInit => JSON.stringify(value)
