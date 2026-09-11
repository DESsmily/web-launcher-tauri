<script setup lang="ts">
import { ui } from "../lib/session";

async function confirm(): Promise<void> {
  const state = ui.confirm;
  ui.confirm = null;
  if (state) await state.onConfirm();
}

function cancel(): void {
  ui.confirm = null;
}
</script>

<template>
  <div v-if="ui.confirm" class="overlay" @click.self="cancel">
    <div class="dialog">
      <h3>{{ ui.confirm.title }}</h3>
      <p>{{ ui.confirm.message }}</p>
      <div class="dialog-actions">
        <button class="btn" @click="cancel">取消</button>
        <button class="btn" :class="ui.confirm.danger ? 'danger' : 'primary'" @click="confirm">
          {{ ui.confirm.confirmText || "确定" }}
        </button>
      </div>
    </div>
  </div>
</template>
