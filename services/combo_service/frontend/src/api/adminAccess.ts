const ADMIN_TOKEN_STORAGE_KEY = 'combo.ops.admin-token.v1'
const ADMIN_TOKEN_FRAGMENT_KEY = 'token'
let inMemoryToken = ''

function readStoredToken(): string {
  try {
    return window.sessionStorage.getItem(ADMIN_TOKEN_STORAGE_KEY)?.trim() || inMemoryToken
  } catch {
    return inMemoryToken
  }
}

function storeToken(token: string): void {
  inMemoryToken = token
  try {
    window.sessionStorage.setItem(ADMIN_TOKEN_STORAGE_KEY, token)
  } catch {
    // The current page can still use the token when browser storage is unavailable.
  }
}

function clearAddressFragment(): void {
  window.history.replaceState(
    window.history.state,
    '',
    `${window.location.pathname}${window.location.search}`,
  )
}

/**
 * Import the bearer token from an unlogged URL fragment, then remove it from
 * the address bar. The token survives refreshes in this tab only.
 */
export function initializeAdminAccess(): string {
  const fragment = window.location.hash.startsWith('#')
    ? window.location.hash.slice(1)
    : ''
  if (!fragment) return readStoredToken()

  const parameters = new URLSearchParams(fragment)
  const token = parameters.get(ADMIN_TOKEN_FRAGMENT_KEY)?.trim() || ''
  clearAddressFragment()
  if (token) storeToken(token)
  return token || readStoredToken()
}

export function getAdminAccessToken(): string {
  return readStoredToken()
}

export function clearAdminAccess(): void {
  inMemoryToken = ''
  try {
    window.sessionStorage.removeItem(ADMIN_TOKEN_STORAGE_KEY)
  } catch {
    // There is no stored credential to clear when session storage is blocked.
  }
}
