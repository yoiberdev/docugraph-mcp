## 📌 Descripción del Cambio
<!-- Explica de forma concisa qué problema resuelve o qué funcionalidad aporta este cambio (máx. 1-2 párrafos). -->

## 🎯 Tipo de Cambio
- [ ] `feat`: Nueva característica o endpoint MCP
- [ ] `fix`: Corrección de error o bug
- [ ] `refactor`: Mejora de código interno sin cambio de comportamiento
- [ ] `perf`: Optimización de memoria, parsing o retrieval
- [ ] `docs`: Documentación, diagramas o guías
- [ ] `test`: Adición o refactorización de pruebas automatizadas

## 🧪 Pruebas y Verificación
<!-- Comandos ejecutados localmente para validar que no hay regresiones -->
- [ ] `cargo check` pasa sin errores
- [ ] `cargo clippy -- -D warnings` pasa limpio
- [ ] `cargo test` pasa al 100%

## 📝 Checklist de Buenas Prácticas
- [ ] Los commits siguen la convención Conventional Commits (`feat:`, `fix:`, etc.)
- [ ] `stdout` se mantiene 100% limpio (los logs van exclusivamente a `stderr`)
- [ ] No se incluyen secretos, archivos temporales ni PDFs con copyright
- [ ] La PR es atómica y concisa (idealmente < 250 líneas)
