import { invoke, isTauri, Channel } from '@tauri-apps/api/core'
import type { AppError } from './types'

export { Channel }
export const nativeAvailable = () => isTauri()
export async function command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!nativeAvailable()) throw { code: 'NATIVE_REQUIRED', message: '请在 DBM 桌面应用中使用数据库功能。当前浏览器仅显示界面。' } satisfies AppError
  try { return await invoke<T>(name, args) }
  catch (error) { throw normalizeError(error) }
}
export function normalizeError(error: unknown): AppError {
  if (typeof error === 'object' && error !== null && 'message' in error) return { ...error, code: 'code' in error ? String(error.code) : 'UNKNOWN', message: String(error.message) } as AppError
  if (typeof error === 'string') {
    try { const parsed = JSON.parse(error); if (parsed.message) return parsed as AppError } catch { /* plain message */ }
    return { code: 'UNKNOWN', message: error }
  }
  return { code: 'UNKNOWN', message: String(error) }
}
