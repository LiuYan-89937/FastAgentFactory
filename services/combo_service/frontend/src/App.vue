<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { RouterView, useRoute } from 'vue-router'
import SiteHeader from '@/components/layout/SiteHeader.vue'
import SiteFooter from '@/components/layout/SiteFooter.vue'
import { useI18n } from '@/i18n'
import { useConfigStore } from '@/stores/config'

const { t } = useI18n()
const config = useConfigStore()
const route = useRoute()
const standalone = computed(() => route.meta.standalone === true)

// Public configuration is shared by all website pages and fails soft.
onMounted(() => {
  void config.ensure()
})
</script>

<template>
  <a href="#main" class="skip-link">{{ t('nav.menu') }}</a>
  <SiteHeader v-if="!standalone" />
  <main id="main" class="app-main">
    <RouterView v-slot="{ Component }">
      <Transition name="route" mode="out-in">
        <component :is="Component" />
      </Transition>
    </RouterView>
  </main>
  <SiteFooter v-if="!standalone" />
</template>

<style scoped>
.app-main {
  flex: 1 0 auto;
}
.route-enter-active,
.route-leave-active {
  transition: opacity var(--dur-base) var(--ease-out);
}
.route-enter-from,
.route-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .route-enter-active,
  .route-leave-active {
    transition: none;
  }
}
</style>
