<script setup lang="ts">
import { onMounted, ref } from 'vue'
import AppReleaseManager from '@/components/admin/AppReleaseManager.vue'
import ErrorReportManager from '@/components/admin/ErrorReportManager.vue'
import BaseButton from '@/components/base/BaseButton.vue'
import StateBlock from '@/components/base/StateBlock.vue'
import { verifyAdminAccess } from '@/api/appReleases'
import { clearAdminAccess, initializeAdminAccess } from '@/api/adminAccess'
import { ApiError, NetworkError } from '@/api/client'
import { useSeo } from '@/composables/useSeo'

const activeSection = ref<'releases' | 'errors'>('releases')
const accessState = ref<'checking' | 'granted' | 'invalid' | 'unavailable'>('checking')
const accessError = ref('')

useSeo(() => ({
  title: '管理控制台',
  description: 'Combo 应用发布与错误上报管理控制台。',
  path: '/ops',
  noindex: true,
}))

async function authorize(): Promise<void> {
  accessState.value = 'checking'
  accessError.value = ''
  if (!initializeAdminAccess()) {
    accessState.value = 'invalid'
    return
  }
  try {
    await verifyAdminAccess()
    accessState.value = 'granted'
  } catch (error) {
    if (error instanceof ApiError && [401, 403].includes(error.status)) {
      clearAdminAccess()
      accessState.value = 'invalid'
      return
    }
    accessState.value = 'unavailable'
    accessError.value = error instanceof NetworkError
      ? '暂时无法连接管理服务，请稍后重试。'
      : '管理服务暂时不可用。'
  }
}

onMounted(authorize)
</script>

<template>
  <div class="admin">
    <section class="admin__hero">
      <div class="container admin__hero-inner">
        <div>
          <span class="eyebrow">Operations</span>
          <h1>管理控制台</h1>
          <p>管理桌面应用版本、安装包、更新日志与用户错误上报。</p>
        </div>
      </div>
    </section>

    <main class="container admin__body">
      <StateBlock v-if="accessState === 'checking'" kind="loading" title="正在验证管理地址" />
      <StateBlock
        v-else-if="accessState === 'invalid'"
        kind="error"
        title="管理地址无效"
        body="请使用包含访问凭证的完整管理地址。"
      />
      <section v-else-if="accessState === 'unavailable'" class="gate">
        <StateBlock kind="error" title="无法连接管理服务" :body="accessError" />
        <BaseButton variant="secondary" @click="authorize">重新连接</BaseButton>
      </section>
      <template v-else>
        <nav class="tabs" aria-label="管理功能">
          <button
            type="button"
            :class="{ tabs__active: activeSection === 'releases' }"
            @click="activeSection = 'releases'"
          >版本发布</button>
          <button
            type="button"
            :class="{ tabs__active: activeSection === 'errors' }"
            @click="activeSection = 'errors'"
          >错误上报</button>
        </nav>
        <AppReleaseManager v-if="activeSection === 'releases'" />
        <ErrorReportManager v-else />
      </template>
    </main>
  </div>
</template>

<style scoped>
.admin__hero {
  padding-block: var(--space-12);
  border-bottom: 1px solid var(--border);
}
.admin__hero-inner {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-6);
}
.admin__hero h1 {
  margin: var(--space-2) 0 0;
  color: var(--text-strong);
  font-size: clamp(2rem, 4vw, 3.5rem);
  line-height: 1;
  letter-spacing: -0.055em;
}
.admin__hero p {
  margin-top: var(--space-3);
  color: var(--text-secondary);
}
.admin__body {
  padding-block: var(--space-8) var(--space-24);
}
.tabs {
  display: inline-flex;
  gap: var(--space-1);
  margin-bottom: var(--space-6);
  padding: 3px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface-subtle);
}
.tabs button {
  height: 38px;
  padding-inline: var(--space-4);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 14px;
  cursor: pointer;
}
.tabs button.tabs__active {
  background: var(--surface);
  color: var(--text-strong);
  box-shadow: var(--shadow-soft);
}
.gate {
  display: grid;
  justify-items: center;
  gap: var(--space-4);
  max-width: 560px;
  margin: var(--space-12) auto;
  text-align: center;
}
</style>
