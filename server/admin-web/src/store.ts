import { defineStore } from 'pinia'
import { api, ApiError, jsonBody, setCSRF, type SessionInfo } from './api'

export const useSessionStore = defineStore('session', {
  state: () => ({ session: null as SessionInfo | null, ready: false }),
  actions: {
    async load() {
      try {
        this.session = await api<SessionInfo>('/admin/api/session')
        setCSRF(this.session.csrfToken)
      } catch (error) {
        if (!(error instanceof ApiError) || error.status !== 401) throw error
        this.session = null
        setCSRF('')
      } finally {
        this.ready = true
      }
    },
    async login(username: string, password: string) {
      const login = await api<{ csrfToken: string }>('/admin/api/login', {
        method: 'POST', body: jsonBody({ username, password })
      })
      setCSRF(login.csrfToken)
      this.session = await api<SessionInfo>('/admin/api/session')
      setCSRF(this.session.csrfToken)
    },
    async logout() {
      await api('/admin/api/logout', { method: 'POST' })
      this.session = null
      setCSRF('')
    },
    can(permission?: string) {
      if (!permission) return true
      return Boolean(this.session?.superAdmin || this.session?.permissions.includes(permission))
    }
  }
})
