"""Summarize measured results, without copying private source-project contents."""
import argparse
import json
from pathlib import Path
import platform
import subprocess


def merge_check(directory, stem, variants):
    paths = []
    for label, text in zip(('ours', 'base', 'theirs'), variants):
        path = directory / f'{stem}.{label}.txt'
        path.write_text(text, encoding='utf-8')
        paths.append(str(path))
    result = subprocess.run(['git', 'merge-file', '-p', *paths], capture_output=True, text=True, encoding='utf-8', errors='replace')
    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr)
    return {'merged_without_conflict': result.returncode == 0, 'conflicts': result.stdout.count('<<<<<<<')}


def compact_entities(text):
    value = json.loads(text)
    entities = value.pop('entities')
    header = json.dumps(value, ensure_ascii=False, separators=(',', ':'))[:-1]
    return header + ',"entities":[\n' + ',\n'.join(json.dumps(e, ensure_ascii=False, separators=(',', ':')) for e in entities) + '\n]}\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, default=Path('target/scene-format-audit'))
    parser.add_argument('--docs', type=Path, default=Path('docs'))
    parser.add_argument('--cpu', default=platform.processor())
    args = parser.parse_args()
    audit = json.loads((args.results / 'audit.json').read_text(encoding='utf-8'))
    rust = json.loads((args.results / 'rust-benchmarks.json').read_text(encoding='utf-8'))
    godot = audit['godot']
    merge_directory = args.results / 'merge-checks'
    merge_directory.mkdir(exist_ok=True)
    variants = [(args.results / f'merge.{name}.scene').read_text(encoding='utf-8') for name in ('ours', 'base', 'theirs')]
    merges = {'property_lines': merge_check(merge_directory, 'property-lines', variants), 'compact_entity_lines': merge_check(merge_directory, 'compact-entities', [compact_entities(v) for v in variants])}
    tscn = (args.results / 'godot-lab/roundtrip.tscn').read_text(encoding='utf-8')
    assert 'power = 9.125' in tscn and '1.125' in tscn
    merges['tscn_property_lines'] = merge_check(merge_directory, 'tscn-properties', [tscn.replace('power = 9.125', 'power = 11.5'), tscn, tscn.replace('1.125', '2.125', 1)])
    inventory = audit['inventory']
    totals = {k: inventory[k] for k in ('files', 'bytes', 'nodes', 'external_resources', 'local_resources')}
    book = next(row for row in inventory['scenes'] if row['file'] == 'components/book/interactive_book.tscn')
    summary = {'method': {'platform': audit['platform'], 'cpu': args.cpu, 'engine': godot['engine']['string'], 'rust': subprocess.check_output([str(Path.home() / '.cargo/bin/rustc.exe'), '--version'], text=True).strip(), 'rust_profile': 'release', 'godot_cache': 'warm OS pages, CACHE_MODE_IGNORE_DEEP, 7 retained samples', 'rust_cache': 'in-memory bytes; warm-up, 21/11 read samples, 5 encode samples', 'caution': 'Other user builds ran on this machine; medians are workload-specific, not a cross-engine ranking.'}, 'source_unchanged': audit['source_unchanged'], 'inventory_totals': totals, 'godot_checks': godot['checks'], 'godot_findings': {k: godot[k] for k in ('unknown_property_accepted', 'unknown_property_preserved', 'saved_in_memory_resource_path')}, 'godot_benchmarks': godot['benchmarks'], 'rust_benchmarks': rust, 'git_merge_checks': merges, 'inline_animation_example': {'scene_bytes': book['bytes'], 'animation_bytes': book['animation_bytes'], 'animations': book['animations']}}
    args.docs.mkdir(exist_ok=True)
    rows = ['| Узлы | Формат | KiB | Загрузка, мс | Создание узлов, мс |', '|---:|---|---:|---:|---:|']
    for row in godot['benchmarks']:
        rows.append(f"| {row['entities']:,} | {row['format']} | {row['bytes']/1024:.1f} | {row['load_median_us']/1000:.3f} | {row['instantiate_median_us']/1000:.3f} |")
    rust_rows = ['| Профиль | Объекты | Операция | KiB | Медиана, мс | P95, мс |', '|---|---:|---|---:|---:|---:|']
    for row in rust:
        if row['entities'] in (10000, 100000):
            rust_rows.append(f"| {row['profile']} | {row['entities']:,} | {row['format']} | {row['bytes']/1024:.1f} | {row['median_us']/1000:.3f} | {row['p95_us']/1000:.3f} |")
    report = f'''# Проверка TSCN и формата сцен

Исследованы {totals['files']} реальных TSCN: {totals['nodes']} деклараций узлов,
{totals['external_resources']} внешних и {totals['local_resources']} внутренних
ресурсов. Это статический инвентарный анализ файлов, а не запуск всех игровых сцен.
Исходные TSCN до и после исследования совпадают по SHA-256. Оригинальные скрипты
не запускались. Godot запускался в отдельном синтетическом проекте.

Использован Godot {summary['method']['engine']}, {args.cpu}, Windows x64;
кодек Rust собран в release. На машине также выполнялись другие пользовательские
сборки. Медианы и P95 описывают конкретный запуск, а не универсальный рейтинг.
Полные пробы сохранены в [JSON](scene-format-benchmarks.json).

## Что у TSCN получилось хорошо

1. Сцена пригодна и для уровня, и для предмета: композиция через PackedScene
   сохраняет исходник и локальные изменения экземпляра. Проверено, что изменение
   базы обновляет обычный экземпляр и сохраняет явное переопределение другого.
2. Материалы, меши и анимации можно вынести в ресурсы или держать внутри файла.
   Кеш ResourceLoader повторно возвращает один ресурс. Это снижает повторные
   загрузки и позволяет явно выбирать совместное использование.
3. Свойства разделены по строкам. Неизменённый экземпляр в тесте сохранялся
   побайтово одинаково. Независимые изменения двух свойств прошли реальное
   трёхстороннее слияние Git без конфликта: {merges['tscn_property_lines']['merged_without_conflict']}.
4. Импортированная модель может оставаться отдельным источником, а сцена — хранить
   настройки поверх неё. В исследованном проекте так устроены персонажи и предметы.
5. Бинарный SCN и его сжатая форма дают отдельный путь оптимизации запуска.

## Самые неприятные места

| Особенность | Что проверено или установлено | Решение в нашем формате |
|---|---|---|
| Разбор большого текста | Загрузка TSCN существенно дороже SCN в этом стенде | Текст для редактирования, таблицы для исполнения |
| Общий изменяемый материал | Изменение одного экземпляра затрагивает второй; local_to_scene изолирует | Явная политика shared/local и устойчивый источник ресурса |
| Неизвестное свойство | Godot принял свойство и потерял его при сохранении | Неизвестные компоненты сохраняются; неизвестные поля схемы отклоняются |
| Ownership | Дочерний узел без owner не вошёл в PackedScene | Явный список авторских сущностей, без неявного owner |
| Пути к узлам | После переименования в runtime сохранённый NodePath перестал разрешаться | Типизированные ссылки по ID с проверкой после композиции |
| Большие встроенные данные | В одном реальном файле анимации занимают {100*book['animation_bytes']/book['bytes']:.1f}% текста | Внешние ресурсы; интернирование одинаковых блоков в бинарном файле |
| Пропуск стандартных свойств | TSCN опускает стандартные значения; изменение стандарта движка/скрипта может менять смысл старого файла | Стандарты Transform закреплены версией; игровые схемы версионирует хост |
| Локальные изменения вложенного объекта | Требуются устойчивые адресаты и правила после удаления/изменения базы | Scoped ID, явная замена компонентов, ошибка при устаревшем адресате |
| Кеш | Повторная load может вернуть прежний объект; сохранённый в памяти PackedScene в тесте имел пустой resource_path | Fingerprint транзитивных сцен и явный шаг компиляции |

Проверка NodePath относится к переименованию во время исполнения. Редактор Godot
умеет обновлять многие ссылки при своих операциях; утверждение, что любое
переименование в Godot ломает сцену, из этого теста не следует. UID ресурса и
локальный ID записи ресурса также выполняют разные задачи.

## Godot: загрузка отдельно от создания объектов

ResourceLoader каждый раз работал с CACHE_MODE_IGNORE_DEEP; файловые страницы
оставались тёплыми. Первый прогон отброшен, затем 7 проб. Сцены содержали обычные
Node3D с трансформациями, без импорта мешей и исполнения игровых скриптов.

{chr(10).join(rows)}

Размер синтетического текста и время его разбора не определяют стоимость
создания узлов, загрузки текстур, GPU-ресурсов или игровых систем.

## Наш кодек: чтение, проверка и подготовка данных

`legacy_typed_parse_only` — typed JSON старой формы с теми же моделями,
трансформациями и именами, без проверки ссылок и файловой системы.
`source_parse_validate` включает полную структурную проверку нового источника.
`packed_checked_view` проверяет бинарные таблицы и предоставляет доступ к ним;
`packed_full_materialize` дополнительно создаёт все изменяемые структуры и
разбирает компоненты. LZ4 включает распаковку и проверку. Операции encode/compile
измеряются отдельно от чтения. Файловая запись и создание ECS-сущностей в этих
пробах Rust не измеряются. Прямые коэффициенты «Rust быстрее Godot» некорректны:
состав работы различается.

{chr(10).join(rust_rows)}

Компактность сама по себе не гарантирует быстрый исходный парсер. Дополнительные
ID, связи и строгая проверка имеют цену. Если нужны все изменяемые объекты и все
компоненты, выигрыш меньше, чем при работе с заимствованными таблицами. LZ4 даёт
компромисс размера и CPU; несжатые таблицы сохраняют непосредственный доступ.

## Проверка Git и дополнительные проверки

На одинаковой сцене независимые правки имени и позиции одного объекта были
переданы `git merge-file`:

- одна строка на сущность: конфликт — {not merges['compact_entity_lines']['merged_without_conflict']};
- отдельные свойства по строкам: без конфликта — {merges['property_lines']['merged_without_conflict']};
- отдельные свойства TSCN: без конфликта — {merges['tscn_property_lines']['merged_without_conflict']}.

Поэтому каноническая запись нового формата разделяет свойства и поля компонентов,
а числовые массивы оставляет компактными. Это увеличивает текст относительно
варианта «весь объект в одну строку», но сохраняет независимые изменения.

Контрактные тесты дополнительно проверяют повторную загрузку одного локатора,
области ID экземпляров, ссылки, удаление поддерева, null и удаление компонента,
конфликты, циклы, устаревшие переопределения, транзитивные fingerprint, лимиты
развёртки, Unicode, u64, неизвестные компоненты, дубли ключей, глубокую иерархию,
каждую точку усечения и каждую однобайтовую порчу небольшого бинарного файла,
поддельные checksum с неверными offset/index/float, случайные некорректные
данные, LZ4 и гонку нескольких согласованных писателей.

## Границы результата

Интерфейс редактора пока редактирует плоские модели, камеры и свет. Новая
библиотека разрешает вложенные сцены и иерархию, но интерфейс таких документов
требует отдельной работы; текущий мост отклоняет их явно. Реестр UID, регистрация
произвольных игровых компонентов, GPU-загрузка, стриминг регионов и сетевой
совместный редактор не реализованы этим кодеком. Peak RSS, холодный SSD/HDD,
консоли и мобильные устройства не измерялись. Аварийное отключение питания
не моделировалось; тестировались revisions, блокировки и атомарная замена.

Воспроизведение и точная структура файла — в [спецификации](scene-format.md).
'''
    verification_path = args.results / 'verification.json'
    if verification_path.exists():
        verification = json.loads(verification_path.read_text(encoding='utf-8'))
        summary['verification'] = verification
        report += '\n## Выполненные проверки реализации\n\n'
        for name, value in verification.items():
            report += f'- {name}: {value}.\n'
    report = report.replace(': True.', ': да.').replace('— True;', '— да;').replace('— True.', '— да.').replace('— False;', '— нет;').replace('— False.', '— нет.')
    (args.docs / 'scene-format-benchmarks.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    (args.docs / 'scene-format-audit.md').write_text(report, encoding='utf-8')
    print(json.dumps({'godot_checks': len(godot['checks']), 'benchmarks': len(rust) + len(godot['benchmarks']), 'git_merge_checks': merges}, ensure_ascii=False))


if __name__ == '__main__':
    main()
