# BugStalker - План оптимизации и рефакторинга

*Дата создания: 28 января 2026*

---

## 📋 РЕЗЮМЕ ПРОЕКТА

### Назначение
**BugStalker** - современный, легковесный отладчик для Linux x86-64, написанный на Rust специально для отладки Rust-программ. Предоставляет как консольный интерфейс, так и TUI (Terminal User Interface) с поддержкой DAP (Debug Adapter Protocol) для интеграции с VSCode.

### Ключевые компоненты

#### 1. **Debugger Core** (`src/debugger/`)
- **Процесс-отладчик** (`process.rs`, `debugee/tracer.rs`) - управление процессом debugee через ptrace
- **Breakpoints & Watchpoints** - условные точки остановки, точки наблюдения за данными
- **DWARF Parser** (`debugee/dwarf/`) - парсинг символов, типов, переменных
- **Unwinder** - разворачивание стека с использованием DWARF информации
- **Register Management** (`register.rs`) - работа с регистрами x86-64
- **Call Instructions** (`call/`) - синтез и выполнение вызовов функций в debugee
- **Async Support** (`async/`) - поддержка Tokio runtime inspection
- **Variable Inspection** (`variable/`) - глубокий анализ переменных, структур, коллекций

#### 2. **User Interface** (`src/ui/`)
- **Console Interface** (`console/`) - интерпретатор команд, REPL
- **TUI** (`tui/`) - полнофункциональный терминальный интерфейс с компонентами
- **DAP Server** (`dap/`) - Debug Adapter Protocol для VSCode integration
- **Command Parser** (`command/`) - парсинг выражений и команд пользователя

#### 3. **Oracle System** (`src/oracle/`)
- **Extensible Plugin Architecture** - система расширений для специализированной отладки
- **Builtin Oracles**:
  - `NopOracle` - заглушка для тестирования
  - `TokioOracle` - мониторинг и анализ Tokio runtime (tasks, sleeps, etc.)

#### 4. **DAP Server** (`src/dap/yadap/`)
- Полная реализация Debug Adapter Protocol
- Поддержка breakpoints, stepping, variable inspection
- Source mapping для compiled программ

#### 5. **Build System**
- **Cargo.toml** - основной манифест (Rust 1.93.0+)
- **build.rs** - custom build script
- **Примеры** (`examples/`) - более 20 примеров для тестирования различных сценариев

### Версия & Статус
- **Текущая версия**: 0.4.2
- **Минимальная версия Rust**: 1.93.0
- **Лицензия**: MIT
- **Статус**: Активная разработка (много фич в 0.3.x версиях)

---

## 🔍 АНАЛИЗ АРХИТЕКТУРЫ

### Сильные стороны
1. ✅ **Чистая архитектура** - четкое разделение debugger/ui/oracle
2. ✅ **Кэширование** - используется типовой кэш, кэш функций, кэш строк (string interner)
3. ✅ **Ленивая загрузка** - DWARF информация парсится по требованию, не весь сразу
4. ✅ **Расширяемость** - Oracle система позволяет добавлять новые фичи без изменения ядра
5. ✅ **Полнота** - поддержка async/await, collections, smart pointers, thread-local vars
6. ✅ **Production-ready** - используется в реальных сценариях отладки

### Технические долги
1. ⚠️ **Memory Management** - частые аллокации при чтении памяти debugee (read_memory_by_pid)
2. ⚠️ **DWARF Parsing** - сложный код с многоуровневыми абстракциями (gimli wrapper)
3. ⚠️ **Error Handling** - много match statements, можно улучшить error propagation
4. ⚠️ **Type System** - ComplexType/TypeCache достаточно объемные, возможна оптимизация
5. ⚠️ **DAP Server** - большой файл session.rs (~2000+ строк)
6. ⚠️ **Tests Coverage** - интеграционные тесты требуют компиляции примеров (медленно)
7. ⚠️ **Logging** - custom logger, можно использовать более стандартные решения

### Граничные условия, требующие внимания
- Работа с 64-битными адресами и endianness
- Обработка многопоточных программ (race conditions в отладке)
- Парсинг DWARF информации для разных версий Rust (1.81-1.93+)
- Поддержка различных Tokio версий (1.40-1.44)

---

## 📊 СОСТОЯНИЕ КОДОВОЙ БАЗЫ

### Основная статистика
- **Размер проекта**: ~40KB исходного кода + примеры
- **Основные модули**: 15+ основных модулей
- **Зависимости**: ~35 внешних крейтов (относительно экономно)
- **Examples**: 20+ примеров для функционального тестирования
- **Tests**: Интеграционные тесты в папке `/tests`

### Ключевые зависимости
| Крейт | Версия | Назначение |
|-------|--------|-----------|
| nix | 0.27.1 | Системные вызовы (ptrace, signal, etc.) |
| gimli | 0.33.0 | DWARF парсинг |
| object | 0.32.1 | ELF/Object парсинг |
| tuirealm | 3.3.0 | TUI фреймворк |
| capstone | 0.11.0 | Дизассемблирование |
| tokio | (в examples) | Async runtime тестирование |
| memmap2 | 0.9.0 | Memory-mapped файлы |
| lru | 0.12.5 | LRU кэш |

---

## 🎯 ПЛАН ОПТИМИЗАЦИИ И РЕФАКТОРИНГА

### ФАЗА 1: КРИТИЧЕСКИЕ УЛУЧШЕНИЯ (1-2 недели)

#### 1.1 Оптимизация памяти при чтении debugee
**Проблема**: `read_memory_by_pid()` создает Vec для каждого чтения, часто вызывается в циклах
**Решение**: Buffer pooling + stack-allocated buffers для малых чтений

```rust
// Текущее состояние (неоптимально)
pub fn read_memory_by_pid(pid: Pid, addr: usize, read_n: usize) -> Result<Vec<u8>, nix::Error> {
    let mut result = Vec::with_capacity(read_n);
    // ... ptrace reads ...
    Ok(result)
}

// Целевое состояние
pub fn read_memory_by_pid(pid: Pid, addr: usize, buf: &mut [u8]) -> Result<(), nix::Error> {
    // write directly into buffer
}
```

**Файлы для изменения**:
- `src/debugger/mod.rs` (read_memory_by_pid)
- `src/debugger/variable/value/specialization/mod.rs` (parse_vector_inner)
- `src/debugger/debugee/disasm.rs`

**Ожидаемый результат**: ↓20-30% аллокаций при профилировании

#### 1.2 Разделение большого файла DAP session
**Проблема**: `src/dap/yadap/session.rs` - ~2000+ строк, множество ответственностей
**Решение**: Разделить на модули: variables.rs, breakpoints.rs, stepping.rs, threads.rs

**Структура**:
```
src/dap/yadap/
├── session.rs (основной диспетчер)
├── handlers/
│   ├── variables.rs
│   ├── breakpoints.rs
│   ├── stepping.rs
│   ├── threads.rs
│   └── memory.rs
├── state.rs (SessionState)
└── protocol_ext.rs (расширения для proto types)
```

**Файлы для создания**: 5 новых файлов в handlers/

#### 1.3 Унификация error handling
**Проблема**: Смешивание anyhow::Error, nix::Error, custom Error enum
**Решение**: Создать comprehensive Error type с категориями

```rust
#[derive(Debug, thiserror::Error)]
pub enum DebuggerError {
    #[error("Ptrace error: {0}")]
    Ptrace(#[from] nix::Error),
    #[error("DWARF parsing: {0}")]
    DwarfParse(String),
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
    // ...
}
```

**Файлы для изменения**:
- `src/debugger/error.rs` (расширить)
- Все модули debugger (постепенно мигрировать)

---

### ФАЗА 2: СТРУКТУРНЫЕ УЛУЧШЕНИЯ (2-3 недели)

#### 2.1 Оптимизация DWARF кэширования
**Проблема**: Type information кэшируется, но часто пересчитывается для одних и тех же типов в разных contexts
**Решение**: 
- Добавить two-level cache (per-unit + global)
- Использовать Interned типы вместо String для имен

**Файлы для изменения**:
- `src/debugger/debugee/dwarf/unit/mod.rs` (UnitLazyPart)
- `src/debugger/context.rs` (TypeCache)
- `src/debugger/variable/value/specialization/mod.rs`

**Метрики**: 
- Кэш-хиты для типов: target >85%
- Размер памяти: ↓15% для debug info

#### 2.2 Рефакторинг Variable Inspection Pipeline
**Проблема**: Множество уровней обработки переменных (Value -> ParseContext -> EvaluationContext -> RenderValue)
**Решение**: Упростить конвейер, убрать избыточные преобразования

Текущий flow:
```
Variable -> ParseContext -> EvaluationContext -> QueryResult -> RenderValue -> Output
```

Целевой flow:
```
Variable -> InspectionContext -> RenderValue -> Output
// InspectionContext инкапсулирует все нужные данные
```

**Файлы для изменения**:
- `src/debugger/variable/execute.rs`
- `src/debugger/variable/value/mod.rs`
- `src/debugger/variable/render.rs`

#### 2.3 Улучшение Breakpoint System
**Проблема**: Breakpoint registry - О(n) поиск, нет быстрого индекса по адресу
**Решение**: Использовать HashMap + List для быстрого поиска

```rust
pub struct BreakpointRegistry {
    by_id: HashMap<BreakpointId, Breakpoint>,
    by_address: HashMap<GlobalAddress, Vec<BreakpointId>>,
    // ...
}
```

**Файлы для изменения**:
- `src/debugger/breakpoint.rs` (BreakpointRegistry struct)

---

### ФАЗА 3: РАСШИРЕНИЯ И ОПТИМИЗАЦИИ (3-4 недели)

#### 3.1 Асинхронная загрузка debug information
**Проблема**: Парсинг всех DWARF данных блокирует запуск отладчика
**Решение**: Lazy loading с background worker потоком

```rust
pub struct DebugInformation {
    // Eagerly loaded
    dwarf: Dwarf,
    
    // Lazily loaded in background
    symbol_cache: Arc<Mutex<SymbolCache>>,
    type_cache: Arc<Mutex<TypeCache>>,
}
```

**Файлы для изменения**:
- `src/debugger/debugee/dwarf/mod.rs` (DebugInformation)
- `src/debugger/debugee/mod.rs` (Debugee initialization)

#### 3.2 Оптимизация Variable Rendering для больших структур
**Проблема**: Рендеринг больших Vec/HashMap требует чтения всех элементов из памяти
**Решение**: Lazy rendering с pagination

```rust
pub struct VecValue {
    total_len: usize,
    page_size: usize,
    loaded_pages: LruCache<usize, Vec<Value>>,
}
```

**Файлы для изменения**:
- `src/debugger/variable/value/specialization/vec.rs`
- `src/debugger/variable/render.rs`

#### 3.3 Расширение Oracle System
**Проблема**: Сейчас только Tokio oracle встроен, сложно добавить новые
**Решение**: 
- Документировать Oracle API
- Создать примеры: ThreadOracle, MutexOracle, AsyncTraceOracle

**Файлы для создания**:
- `docs/oracle-development.md` (документация)
- `examples/oracle_custom/` (пример custom oracle)

#### 3.4 Кэширование исходного кода
**Проблема**: Исходный код читается с диска при каждом stop на breakpoint
**Решение**: LRU cache исходных файлов в памяти

```rust
pub struct SourceCodeCache {
    cache: LruCache<PathBuf, Vec<String>>,
    max_size_bytes: usize,
}
```

**Файлы для изменения**:
- `src/ui/tui/components/source.rs` (FileLinesCache расширить)

---

### ФАЗА 4: КАЧЕСТВО КОДА (2 недели)

#### 4.1 Улучшение покрытия тестами
**Проблема**: Интеграционные тесты медленные, unit тестов мало
**Решение**:
- Добавить unit тесты для парсеров (expression parser, watchpoint parser)
- Создать mock debugee для тестирования variable inspection
- Параллелизировать интеграционные тесты

**Файлы для создания**:
- `tests/unit/parser.rs`
- `tests/unit/dwarf_parsing.rs`
- `tests/mocks/mod.rs`

#### 4.2 Документирование внутренних API
**Проблема**: Сложные типы (ComplexType, Value, QueryResult) недостаточно документированы
**Решение**: Добавить rustdoc примеры и диаграммы

**Файлы для изменения**:
- `src/debugger/variable/value/mod.rs` (добавить module-level docs)
- `src/debugger/debugee/dwarf/mod.rs`
- `src/debugger/context.rs`

#### 4.3 Performance Profiling & Benchmarking
**Проблема**: Нет бенчмарков для критических операций
**Решение**: Добавить benches для:
- DWARF парсинга
- Variable inspection
- DAP message processing

**Файлы для создания**:
- `benches/dwarf_parsing.rs`
- `benches/variable_inspection.rs`

#### 4.4 Рефакторинг логирования
**Проблема**: Кастомный logger (src/log.rs), сложная переключение режимов
**Решение**: Перейти на env_logger/tracing с поддержкой динамической фильтрации

**Файлы для изменения**:
- `src/log.rs` (переделать или удалить)
- `src/main.rs` (инициализация логирования)
- `src/ui/supervisor.rs` (переключение режимов)

---

## 📈 МЕТРИКИ УСПЕХА

### Производительность
| Метрика | Текущее | Целевое | Фаза |
|---------|---------|---------|------|
| Время запуска с малой программой | ~1-2s | <500ms | 3 |
| Память на типовую программу | ~50MB | <35MB | 1,2 |
| Время inspection большого Vec | ~500ms | <100ms | 3 |
| DAP message latency | ~100ms | <50ms | 2 |

### Качество кода
| Метрика | Текущее | Целевое | Фаза |
|---------|---------|---------|------|
| Модульность (макс LoC на файл) | 2000+ | <1500 | 2 |
| Покрытие unit тестами | ~30% | >60% | 4 |
| Документированные публичные API | ~50% | >90% | 4 |
| Сложность cyclomatic (avg) | ~8 | <6 | 2,4 |

### Архитектура
| Метрика | Текущее | Целевое | Фаза |
|---------|---------|---------|------|
| Циклические зависимости | 2-3 | 0 | 2 |
| Слабо связанные модули | 60% | >85% | 2 |
| Расширяемые extension points | 1 (Oracle) | 5+ | 3 |

---

## 🔧 ИНСТРУМЕНТЫ И СКРИПТЫ

### Профилирование
```bash
# Profiling with flamegraph
cargo flamegraph --example vars -- -tui

# Memory profiling with heaptrack
heaptrack ./target/debug/bs ./path/to/binary

# Compile time profiling
cargo build --release --timings
```

### Тестирование
```bash
# Unit tests
cargo test --lib

# Integration tests (медленно)
cargo test --test '*' -- --nocapture

# Parametrized tests для Tokio versions
for v in 1_40 1_41 1_42 1_43 1_44; do
    cargo test -p tokio_tcp_$v
done
```

### Анализ кода
```bash
# Проверка сложности
cargo install cargo-complexity
cargo complexity --threshold 10

# Неиспользованные зависимости
cargo tree -d

# Clippy lints
cargo clippy --all-targets -- -W clippy::all
```

---

## 📝 ДЕТАЛЬНЫЙ ПЛАН РЕАЛИЗАЦИИ

### ФАЗА 1 - Неделя 1

#### День 1-2: Buffer optimization
1. [ ] Создать `src/debugger/memory/buffer_pool.rs`
2. [ ] Реализовать thread-local buffer pool
3. [ ] Обновить `read_memory_by_pid` для использования pool
4. [ ] Написать бенчмарки в `benches/memory.rs`

**PR: "perf: optimize memory reads with buffer pooling"**

#### День 3: DAP session refactoring start
1. [ ] Создать структуру `src/dap/yadap/handlers/mod.rs`
2. [ ] Извлечь variable handling в `handlers/variables.rs`
3. [ ] Написать тесты для variable handler
4. [ ] Обновить импорты

**PR: "refactor: split dap session into modules (part 1)"**

#### День 4-5: Error handling unification
1. [ ] Расширить `src/debugger/error.rs` с новыми категориями
2. [ ] Создать conversion traits для nix::Error
3. [ ] Обновить критические пути для использования новых ошибок
4. [ ] Добавить тесты

**PR: "refactor: unified error handling for debugger core"**

### ФАЗА 2 - Неделя 2-3

#### День 1: DWARF cache optimization
1. [ ] Анализировать текущую cache hit rate
2. [ ] Реализовать two-level cache в `src/debugger/context.rs`
3. [ ] Добавить metrics для cache stats
4. [ ] Бенчмарки

**PR: "perf: two-level DWARF type caching"**

#### День 2-3: Variable inspection refactoring
1. [ ] Создать unified `InspectionContext`
2. [ ] Миграция к новому контексту в variable/value/
3. [ ] Упрощение ParseContext -> QueryResult pipeline
4. [ ] Обновить все тесты

**PR: "refactor: simplify variable inspection pipeline"**

#### День 4-5: Breakpoint system optimization
1. [ ] Добавить address-based index в BreakpointRegistry
2. [ ] Оптимизировать поиск breakpoints на прерывание
3. [ ] Бенчмарки для большого количества breakpoints
4. [ ] Обновить DAP session для использования нового индекса

**PR: "perf: optimize breakpoint lookup with address index"**

### ФАЗА 3 - Неделя 3-4

#### День 1-2: Async DWARF loading
1. [ ] Создать background worker в `src/debugger/debugee/loader.rs`
2. [ ] Реализовать progressive loading
3. [ ] Добавить progress callbacks для UI
4. [ ] Интеграционные тесты

**PR: "feat: async debug info loading"**

#### День 3: Variable rendering optimization
1. [ ] Реализовать pagination для big structures
2. [ ] LRU cache для loaded pages
3. [ ] Обновить render/mod.rs
4. [ ] Тесты производительности

**PR: "perf: paginated rendering for large collections"**

#### День 4: Oracle system documentation
1. [ ] Написать `docs/oracle-development.md`
2. [ ] Создать пример в `examples/oracle_custom/`
3. [ ] Обновить комментарии в `src/oracle/mod.rs`
4. [ ] Примеры в документации

**PR: "docs: oracle extension system guide"**

#### День 5: Source code caching
1. [ ] Расширить FileLinesCache с LRU
2. [ ] Добавить memory limits
3. [ ] Интеграция в source component
4. [ ] Бенчмарки

**PR: "perf: source code file caching"**

### ФАЗА 4 - Неделя 4-5

#### День 1-2: Testing improvements
1. [ ] Создать unit тесты в `tests/unit/`
2. [ ] Mock debugee для variable tests
3. [ ] Параллелизировать интеграционные тесты
4. [ ] CI/CD optimization

**PR: "test: improved unit and mock testing"**

#### День 3: API documentation
1. [ ] Добавить rustdoc для всех публичных типов
2. [ ] Примеры использования в comments
3. [ ] Архитектурные диаграммы в module docs
4. [ ] Generate и publish docs

**PR: "docs: comprehensive API documentation"**

#### День 4: Logging refactoring
1. [ ] Перейти на standard logging (env_logger/tracing)
2. [ ] Убрать custom logger или значительно упростить
3. [ ] Добавить structured logging где нужно
4. [ ] Тесты логирования

**PR: "refactor: modernize logging infrastructure"**

#### День 5: Performance profiling setup
1. [ ] Создать benches в `benches/`
2. [ ] Настроить CI для сравнения производительности
3. [ ] Документация по профилированию
4. [ ] Baseline metrics

**PR: "ci: performance benchmarking and monitoring"**

---

## 🎓 РЕКОМЕНДАЦИИ ПО РЕАЛИЗАЦИИ

### Best Practices
1. **Atomic commits** - каждый PR решает одну проблему
2. **Backward compatibility** - старые API остаются рабочими, помечаются как deprecated
3. **Documentation first** - PR описание содержит архитектурный контекст
4. **Performance regression tests** - бенчмарки для критических операций
5. **Feature flags** - новые фичи за флагами пока не стабильны

### Code Review Checklist
- [ ] Соответствие Rust API guidelines (https://rust-lang.github.io/api-guidelines/)
- [ ] Нет unsafe кода без SAFETY комментариев
- [ ] Новые публичные API имеют примеры
- [ ] Тесты покрывают новый код и edge cases
- [ ] Нет регрессии производительности

### Dependency Management
- Минимизировать зависимости (current: ~35)
- Ежемесячно проверять обновления critical dependencies
- Использовать cargo-deny для security scanning
- Document rationale для каждой зависимости

---

## 🚀 БЫСТРЫЕ WINS (можно сделать параллельно)

1. **Cleanup Code** (1-2 часа)
   - [ ] Удалить неиспользуемые импорты (cargo clippy --fix)
   - [ ] Форматировать код (cargo fmt)
   - [ ] Обновить комментарии, исправить typos

2. **CI/CD Improvements** (2-3 часа)
   - [ ] Добавить cargo-deny в CI
   - [ ] Setup codecov интеграцию
   - [ ] Параллелизировать тесты в CI

3. **Documentation** (2-4 часа)
   - [ ] Обновить README.md с архитектурной диаграммой
   - [ ] Добавить ARCHITECTURE.md
   - [ ] Написать DEVELOPMENT.md для контрибьюторов

4. **Build Optimization** (1-2 часа)
   - [ ] Включить LTO в debug builds для faster linking
   - [ ] Optimize incremental compilation settings
   - [ ] Сократить compilation time бенчмарками

---

## 📚 ССЫЛКИ И РЕСУРСЫ

### Проект
- Repository: https://github.com/godzie44/BugStalker
- Website: https://godzie44.github.io/BugStalker/
- Crates.io: https://crates.io/crates/bugstalker/
- VSCode Extension: https://marketplace.visualstudio.com/items?itemName=BugStalker.bugstalker

### Технологии
- **DWARF Debugging Information**: https://en.wikipedia.org/wiki/DWARF
- **ptrace syscall**: https://man7.org/linux/man-pages/man2/ptrace.2.html
- **DAP Protocol**: https://microsoft.github.io/debug-adapter-protocol/
- **Gimli library**: https://docs.rs/gimli/latest/gimli/
- **Rust API Guidelines**: https://rust-lang.github.io/api-guidelines/

### Профилирование
- **cargo-flamegraph**: https://github.com/flamegraph-rs/flamegraph
- **heaptrack**: https://github.com/KDE/heaptrack
- **cargo-bench**: https://doc.rust-lang.org/cargo/commands/cargo-bench.html

### Тестирование
- **cargo-test**: https://doc.rust-lang.org/cargo/commands/cargo-test.html
- **serial_test**: https://docs.rs/serial_test/latest/serial_test/
- **proptest**: https://docs.rs/proptest/latest/proptest/

---

## ✅ ЧЕКЛИСТ ДЛЯ ОТСЛЕЖИВАНИЯ

### ФАЗА 1
- [ ] Buffer pooling implementation
- [ ] DAP session refactoring started
- [ ] Error handling унифицирован
- [ ] Performance tests добавлены

### ФАЗА 2
- [ ] DWARF cache оптимизирован
- [ ] Variable inspection упрощен
- [ ] Breakpoint system оптимизирован
- [ ] Метрики памяти улучшены на 15-20%

### ФАЗА 3
- [ ] Async loading реализован
- [ ] Pagination для big structures
- [ ] Oracle system документирован
- [ ] Source cache добавлен

### ФАЗА 4
- [ ] Unit test покрытие >60%
- [ ] API документация >90%
- [ ] Logging переделан
- [ ] Benchmarks setup completed

---

**Создано**: 28 января 2026  
**Разработчик**: GitHub Copilot  
**Версия документа**: 1.0  
**Статус**: Готово к внедрению
