//! The UI in the user's language: English text in the code, looked up in a table of
//! translations at the moment it is shown.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    /// The stored and system form: `en`, `ru`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    /// From a code or a locale like `ru-RU`; `None` for languages without a translation.
    pub fn from_code(code: &str) -> Option<Self> {
        let base = code.split(['-', '_']).next()?.to_ascii_lowercase();
        match base.as_str() {
            "en" => Some(Lang::En),
            "ru" => Some(Lang::Ru),
            _ => None,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Ru,
        _ => Lang::En,
    }
}

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
    pb_platform::dialog::set_web_texts(pb_platform::dialog::WebTexts {
        ok_saves: tr("OK: save it.").into(),
        discard: tr("Close the board without saving it?").into(),
        board_name: tr("Board name").into(),
    });
}

/// `en` in the current language; itself when there is no translation.
pub fn tr(en: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Ru => russian().get(en).copied().unwrap_or(en),
    }
}

/// [`tr`] of `template` with each `{}` in it replaced by the next of `args`.
pub fn trf(template: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut parts = tr(template).split("{}");
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for part in parts {
        if let Some(arg) = args.next() {
            out.push_str(&arg.to_string());
        }
        out.push_str(part);
    }
    out
}

/// Whether `en` has a Russian translation (for tests of texts kept in tables).
#[cfg(test)]
pub fn translated(en: &str) -> bool {
    russian().contains_key(en)
}

fn russian() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| RU.iter().copied().collect())
}

/// English UI text and its Russian translation.
const RU: &[(&str, &str)] = &[
    // The board menu.
    ("read-only", "только чтение"),
    ("File", "Файл"),
    ("Read-only", "Только чтение"),
    (
        "Protect the board: only looking and commenting",
        "Защитить доску: только просмотр и комментарии",
    ),
    ("Frames", "Фреймы"),
    (
        "No frames yet: draw one with F",
        "Фреймов пока нет: нарисуйте с помощью F",
    ),
    ("Untitled frame", "Фрейм без названия"),
    ("Filter", "Фильтр"),
    ("Filter ●", "Фильтр ●"),
    ("Present", "Показ"),
    (
        "Show the frames one by one (F5)",
        "Показать фреймы по очереди (F5)",
    ),
    (
        "Add frames to present them",
        "Добавьте фреймы, чтобы их показать",
    ),
    ("Talktracks", "Записи"),
    ("History", "История"),
    ("Vote", "Голосование"),
    ("Timer", "Таймер"),
    ("{} min", "{} мин"),
    ("Templates", "Шаблоны"),
    (
        "Start from a ready-made layout",
        "Начать с готовой заготовки",
    ),
    ("Comments ({})", "Комментарии ({})"),
    ("Comments", "Комментарии"),
    ("Find", "Найти"),
    ("Find on the board (Ctrl+F)", "Поиск по доске (Ctrl+F)"),
    ("Keyboard shortcuts (F1)", "Горячие клавиши (F1)"),
    ("New board", "Новая доска"),
    ("Open…", "Открыть…"),
    ("Download board", "Скачать доску"),
    ("Share", "Поделиться"),
    ("Everyone follows your view", "Все следят за вашим видом"),
    ("Following {}", "Вы следите за: {}"),
    ("Stop following", "Перестать следить"),
    ("Follow: see what they see", "Следить: видеть то же, что он"),
    ("(presenting)", "(ведёт показ)"),
    ("Stop leading", "Перестать вести"),
    ("Bring everyone to me", "Собрать всех у себя"),
    (
        "Everyone on the board follows your view until they move",
        "Все на доске следят за вашим видом, пока сами не сдвинутся",
    ),
    ("Your name", "Ваше имя"),
    (
        "Shown to others next to your pointer, comments and votes",
        "Видно другим рядом с вашим указателем, комментариями и голосами",
    ),
    (
        "Work on this board together through a server",
        "Работать над доской вместе через сервер",
    ),
    ("Try again", "Повторить"),
    (
        "You can look at this board but not change it.",
        "Эту доску можно смотреть, но не менять.",
    ),
    ("Here now:", "Сейчас здесь:"),
    ("Link to edit", "Ссылка для правки"),
    (
        "Whoever has it can change the board",
        "У кого она есть, может менять доску",
    ),
    ("Link to look", "Ссылка для просмотра"),
    (
        "Whoever has it can only look",
        "У кого она есть, может только смотреть",
    ),
    ("Stop sharing", "Перестать делиться"),
    (
        "Keep working on this copy alone",
        "Продолжить работать с этой копией одному",
    ),
    (
        "Put this board on a server to work on it together: everyone with the link sees changes as they are made.",
        "Выложите доску на сервер, чтобы работать вместе: все, у кого есть ссылка, видят правки сразу.",
    ),
    ("Making the board…", "Создаю доску…"),
    ("Share this board", "Поделиться этой доской"),
    (
        "Or open a board someone shared with you:",
        "Или откройте доску, которой с вами поделились:",
    ),
    ("That is not a server address.", "Это не адрес сервера."),
    (
        "The server's answer is not understood.",
        "Ответ сервера непонятен.",
    ),
    (
        "That is not a link to a shared board.",
        "Это не ссылка на общую доску.",
    ),
    ("Connecting…", "Подключаюсь…"),
    ("Online: only you here", "В сети: здесь только вы"),
    ("Offline", "Не в сети"),
    (
        "Could not share the board: {}",
        "Не удалось поделиться доской: {}",
    ),
    ("Online: {} more here", "В сети: здесь ещё {}"),
    (
        "Offline: {}. Changes are kept and sent once back.",
        "Не в сети: {}. Правки сохранятся и отправятся, когда связь вернётся.",
    ),
    (
        "Save the board as a .pboard file on this computer",
        "Сохранить доску файлом .pboard на этот компьютер",
    ),
    ("Boards in this browser", "Доски в этом браузере"),
    (
        "Delete this board from the browser",
        "Удалить эту доску из браузера",
    ),
    (
        "It is open: open another board first",
        "Она открыта: сначала откройте другую доску",
    ),
    (
        "Delete the board \"{}\" from this browser?",
        "Удалить доску «{}» из этого браузера?",
    ),
    ("OK: save it.", "OK — сохранить."),
    (
        "Close the board without saving it?",
        "Закрыть доску без сохранения?",
    ),
    ("Board name", "Название доски"),
    ("Save", "Сохранить"),
    ("Save as…", "Сохранить как…"),
    ("Insert file…", "Вставить файл…"),
    ("Diagram from text…", "Диаграмма из текста…"),
    ("Export…", "Экспорт…"),
    ("Print…", "Печать…"),
    // Toolbars.
    ("Select (V)", "Выделение (V)"),
    ("Comment (C)", "Комментарий (C)"),
    ("Shapes (S, R, O)", "Фигуры (S, R, O)"),
    ("Pen (P)", "Ручка (P)"),
    ("Marker (M)", "Маркер (M)"),
    ("Eraser (E)", "Ластик (E)"),
    ("Text (T)", "Текст (T)"),
    ("Sticky note (N)", "Стикер (N)"),
    ("Connector (L)", "Коннектор (L)"),
    (
        "Image (or drop, paste)",
        "Картинка (или перетащите, вставьте)",
    ),
    ("Undo (Ctrl+Z)", "Отменить (Ctrl+Z)"),
    ("Redo (Ctrl+Y)", "Повторить (Ctrl+Y)"),
    ("Frame (F)", "Фрейм (F)"),
    ("Kanban (K)", "Канбан (K)"),
    ("Mind map", "Майнд-карта"),
    ("Markdown document (D)", "Markdown-документ (D)"),
    ("Start arrow", "Стрелка в начале"),
    ("Arrowhead at the start", "Наконечник в начале"),
    ("End arrow", "Стрелка в конце"),
    ("Arrowhead at the end", "Наконечник в конце"),
    ("Reset path", "Сбросить путь"),
    (
        "Drop the bends set by hand",
        "Убрать изгибы, заданные вручную",
    ),
    // Shapes.
    ("Basic", "Основные"),
    ("Rectangle", "Прямоугольник"),
    ("Rounded rectangle", "Скруглённый прямоугольник"),
    ("Ellipse", "Эллипс"),
    ("Diamond", "Ромб"),
    ("Triangle", "Треугольник"),
    ("Star", "Звезда"),
    ("Arrow", "Стрелка"),
    ("Double arrow", "Двойная стрелка"),
    ("Cloud", "Облако"),
    ("Hexagon", "Шестиугольник"),
    ("Parallelogram", "Параллелограмм"),
    ("Trapezoid", "Трапеция"),
    ("Flowchart", "Блок-схема"),
    ("Start / end", "Начало / конец"),
    ("Process", "Процесс"),
    ("Decision", "Решение"),
    ("Data (input / output)", "Данные (ввод / вывод)"),
    ("Document", "Документ"),
    ("Predefined process", "Предопределённый процесс"),
    ("Manual input", "Ручной ввод"),
    ("Manual operation", "Ручная операция"),
    ("Preparation", "Подготовка"),
    ("Database", "База данных"),
    ("Delay", "Задержка"),
    ("Off-page connector", "Межстраничная ссылка"),
    ("UML", "UML"),
    ("Class", "Класс"),
    ("Actor", "Актёр"),
    ("Use case", "Прецедент"),
    ("State", "Состояние"),
    ("Note", "Примечание"),
    ("Package", "Пакет"),
    ("Entity relationship", "Сущность — связь"),
    ("Entity", "Сущность"),
    ("Relationship", "Связь"),
    ("Attribute", "Атрибут"),
    ("Table", "Таблица"),
    ("BPMN", "BPMN"),
    ("Task", "Задача"),
    ("Event", "Событие"),
    ("Gateway", "Шлюз"),
    ("Data object", "Объект данных"),
    ("Data store", "Хранилище данных"),
    ("Pool", "Пул"),
    ("Network", "Сеть"),
    ("Server", "Сервер"),
    ("Computer", "Компьютер"),
    ("User", "Пользователь"),
    // View controls.
    ("Untitled", "Без названия"),
    ("Dots", "Точки"),
    ("Lines", "Линии"),
    ("No grid", "Без сетки"),
    ("Grid style (G)", "Вид сетки (G)"),
    ("Snap to grid", "Привязка к сетке"),
    ("Shift+G", "Shift+G"),
    ("Dark", "Тёмная"),
    ("Dark theme", "Тёмная тема"),
    ("Language: English / Russian", "Язык: английский / русский"),
    ("View", "Вид"),
    ("Interface size", "Размер интерфейса"),
    (
        "Ctrl+Alt+= / Ctrl+Alt+- / Ctrl+Alt+0",
        "Ctrl+Alt+= / Ctrl+Alt+- / Ctrl+Alt+0",
    ),
    ("High contrast", "Высокий контраст"),
    (
        "Black or white text and clear outlines",
        "Чёрный или белый текст и чёткие рамки",
    ),
    (
        "Interface size: larger / smaller / normal",
        "Размер интерфейса: больше / меньше / обычный",
    ),
    ("Map", "Карта"),
    ("Minimap", "Миникарта"),
    ("Layers", "Слои"),
    ("Layers of the board", "Слои доски"),
    ("Hidden ({})", "Скрытые ({})"),
    (
        "Show hidden objects faded, to pick and show them",
        "Показать скрытые объекты бледными, чтобы выбрать и вернуть их",
    ),
    ("Zoom out (Ctrl+−)", "Уменьшить (Ctrl+−)"),
    ("Reset zoom (Ctrl+0)", "Масштаб 100% (Ctrl+0)"),
    ("Zoom in (Ctrl+=)", "Увеличить (Ctrl+=)"),
    // The selection bar.
    ("Copy", "Копировать"),
    ("Ctrl+C", "Ctrl+C"),
    ("Unlock", "Разблокировать"),
    ("Ctrl+Shift+L", "Ctrl+Shift+L"),
    ("Aa", "Aa"),
    (
        "Text color, size and alignment",
        "Цвет, размер и выравнивание текста",
    ),
    ("Recognizing…", "Распознаю…"),
    ("To text", "В текст"),
    (
        "Turn the handwriting into typed text",
        "Превратить рукописное в печатный текст",
    ),
    ("Calculate", "Вычислить"),
    (
        "Solve a written expression such as 12 + 7 =",
        "Решить написанное выражение, например 12 + 7 =",
    ),
    ("+ Child", "+ Подтема"),
    ("Add a subtopic (Tab)", "Добавить подтему (Tab)"),
    ("+ Sibling", "+ Тема"),
    (
        "Add a topic after this one (Enter)",
        "Добавить тему после этой (Enter)",
    ),
    ("Expand", "Развернуть"),
    ("Collapse", "Свернуть"),
    ("Show or hide the subtopics", "Показать или скрыть подтемы"),
    ("Delete topic", "Удалить тему"),
    (
        "Delete it with its subtopics (Delete)",
        "Удалить вместе с подтемами (Delete)",
    ),
    ("Update preview", "Обновить превью"),
    (
        "Fetch the page's title and description again",
        "Заново получить заголовок и описание страницы",
    ),
    ("Crop", "Кадрировать"),
    ("Previous page (PageUp)", "Предыдущая страница (PageUp)"),
    ("Next page (PageDown)", "Следующая страница (PageDown)"),
    (
        "Page: type a number or drag",
        "Страница: введите номер или потяните",
    ),
    ("of {}", "из {}"),
    (
        "Next / previous page of a selected PDF book",
        "Следующая / предыдущая страница выделенной PDF-книги",
    ),
    (
        "Could not turn the page",
        "Не удалось перелистнуть страницу",
    ),
    (
        "Double click or Enter; Enter or a click outside to finish, Esc to cancel",
        "Двойной щелчок или Enter; Enter или щелчок мимо — готово, Esc — отмена",
    ),
    ("Link", "Ссылка"),
    (
        "Link to a page or another object",
        "Ссылка на страницу или другой объект",
    ),
    ("React", "Реакция"),
    ("Add a reaction", "Добавить реакцию"),
    ("Tags", "Теги"),
    ("Tags on the selection", "Теги выделенного"),
    ("Opacity", "Прозрачность"),
    ("Arrange", "Разложить"),
    (
        "Lay out in a grid or in groups",
        "Разложить сеткой или группами",
    ),
    ("Group", "Сгруппировать"),
    ("Ctrl+G", "Ctrl+G"),
    ("Ungroup", "Разгруппировать"),
    ("Ctrl+Shift+G", "Ctrl+Shift+G"),
    ("Lock", "Заблокировать"),
    ("Layer", "Слой"),
    ("Move to another layer", "Перенести на другой слой"),
    ("Show", "Показать"),
    ("Hide", "Скрыть"),
    ("Ctrl+Shift+H", "Ctrl+Shift+H"),
    ("Front", "Вперёд"),
    ("Ctrl+Shift+]", "Ctrl+Shift+]"),
    ("Back", "Назад"),
    ("Ctrl+Shift+[", "Ctrl+Shift+["),
    ("Duplicate", "Дублировать"),
    ("Ctrl+D", "Ctrl+D"),
    ("Delete", "Удалить"),
    ("Fill color", "Цвет заливки"),
    ("Outline", "Контур"),
    ("Color", "Цвет"),
    ("Auto", "Авто"),
    (
        "As large as fits the shape",
        "Как можно крупнее, чтобы влезло в фигуру",
    ),
    (
        "Font size; text that does not fit the shape shrinks",
        "Размер шрифта; не влезающий в фигуру текст уменьшается",
    ),
    ("Left", "Слева"),
    ("Center", "По центру"),
    ("Right", "Справа"),
    ("Top", "Сверху"),
    ("Middle", "Посередине"),
    ("Bottom", "Снизу"),
    ("Open", "Открыть"),
    ("Remove", "Убрать"),
    (
        "Web address or copied object link",
        "Веб-адрес или скопированная ссылка на объект",
    ),
    ("Set", "Задать"),
    ("Link to a frame:", "Ссылка на фрейм:"),
    ("Click an object…", "Щёлкните объект…"),
    ("Pick on board", "Выбрать на доске"),
    (
        "Then click the object to link to; Esc cancels",
        "Затем щёлкните объект, на который ссылаться; Esc — отмена",
    ),
    ("Copy link", "Копировать ссылку"),
    (
        "Copy a link to this object, to paste into another one's Link",
        "Скопировать ссылку на этот объект, чтобы вставить в «Ссылку» другого",
    ),
    ("Grid", "Сеткой"),
    (
        "In a grid, in reading order (Alt+G)",
        "Сеткой, в порядке чтения (Alt+G)",
    ),
    ("By color", "По цвету"),
    (
        "In groups by fill color, with a heading each",
        "Группами по цвету заливки, у каждой заголовок",
    ),
    ("By tag", "По тегу"),
    (
        "In groups by tag, with a heading each",
        "Группами по тегу, у каждой заголовок",
    ),
    ("New tag", "Новый тег"),
    ("Add", "Добавить"),
    // Tables and kanban.
    ("+ Row", "+ Строка"),
    ("Add a row at the bottom", "Добавить строку снизу"),
    ("+ Column", "+ Столбец"),
    ("Add a column at the right", "Добавить столбец справа"),
    ("− Row", "− Строка"),
    ("Remove the bottom row", "Убрать нижнюю строку"),
    ("− Column", "− Столбец"),
    ("Remove the right column", "Убрать правый столбец"),
    ("Row ↑", "Строка ↑"),
    ("Insert a row above", "Вставить строку выше"),
    ("Row ↓", "Строка ↓"),
    ("Insert a row below", "Вставить строку ниже"),
    ("Column ←", "Столбец ←"),
    ("Insert a column to the left", "Вставить столбец слева"),
    ("Column →", "Столбец →"),
    ("Insert a column to the right", "Вставить столбец справа"),
    ("Delete rows", "Удалить строки"),
    ("Delete the selected rows", "Удалить выбранные строки"),
    ("Delete columns", "Удалить столбцы"),
    ("Delete the selected columns", "Удалить выбранные столбцы"),
    ("Unmerge", "Разъединить"),
    ("Split the merged cells", "Разделить объединённые ячейки"),
    ("Merge", "Объединить"),
    (
        "Join the selected cells into one",
        "Объединить выбранные ячейки в одну",
    ),
    ("Delete card", "Удалить карточку"),
    (
        "Delete the selected card (Delete)",
        "Удалить выбранную карточку (Delete)",
    ),
    // The right-click menu.
    ("Select all", "Выделить всё"),
    ("Copy link to object", "Копировать ссылку на объект"),
    ("Paste here", "Вставить сюда"),
    ("Add sticky note here", "Добавить стикер сюда"),
    ("Add text here", "Добавить текст сюда"),
    ("Exit focus", "Выйти из фокуса"),
    ("Focus on this view", "Фокус на этой области"),
    ("Edit text", "Править текст"),
    ("Cut", "Вырезать"),
    ("Paste", "Вставить"),
    ("Bring to front", "На передний план"),
    ("Send to back", "На задний план"),
    ("Focus on selection", "Фокус на выделенном"),
    // Comments.
    (
        "No comments yet: use the comment tool (C)",
        "Комментариев пока нет: используйте инструмент «Комментарий» (C)",
    ),
    ("Show resolved", "Показывать решённые"),
    ("New comment", "Новый комментарий"),
    ("Close (Esc)", "Закрыть (Esc)"),
    ("Delete the thread", "Удалить обсуждение"),
    ("Reopen", "Открыть снова"),
    ("Mark it open again", "Снова сделать открытым"),
    ("Resolve", "Решено"),
    (
        "Mark it done; it hides unless resolved ones show",
        "Отметить решённым; скрывается, если решённые не показываются",
    ),
    (
        "Write a comment… @name mentions someone",
        "Напишите комментарий… @имя — упомянуть",
    ),
    ("Comment", "Комментарий"),
    ("Post", "Отправить"),
    ("Reply", "Ответить"),
    ("Ctrl+Enter", "Ctrl+Enter"),
    // Voting and the timer.
    ("Votes per person:", "Голосов на человека:"),
    ("{} of {} votes left", "Осталось голосов: {} из {}"),
    ("End voting", "Завершить голосование"),
    ("Results", "Итоги"),
    ("Clear votes", "Сбросить голоса"),
    ("Voting", "Голосование"),
    (
        "{} of {} votes left · click an object to vote, Shift+click to take one back",
        "Осталось голосов: {} из {} · щелчок по объекту — голос, Shift+щелчок — забрать",
    ),
    ("Voting results", "Итоги голосования"),
    ("No votes yet", "Голосов пока нет"),
    ("Time's up", "Время вышло"),
    ("Pause", "Пауза"),
    ("Resume", "Продолжить"),
    ("Stop the clock", "Остановить отсчёт"),
    ("Run the clock on", "Продолжить отсчёт"),
    ("Start over", "Сначала"),
    ("Close", "Закрыть"),
    // Layers.
    ("+ Add", "+ Добавить"),
    ("New layer on top", "Новый слой сверху"),
    ("Visible", "Видимый"),
    (
        "New objects go to the selected layer; double click to rename",
        "Новые объекты попадают на выбранный слой; двойной щелчок — переименовать",
    ),
    (
        "Nothing on a locked layer can be changed",
        "На заблокированном слое ничего нельзя изменить",
    ),
    (
        "Remove the layer and everything on it",
        "Удалить слой со всем, что на нём",
    ),
    // Talktracks and versions.
    ("● Record talktrack", "● Записать"),
    (
        "Record your voice while you move around the board",
        "Записать голос, пока вы ходите по доске",
    ),
    ("no microphone", "нет микрофона"),
    ("Stop", "Стоп"),
    ("Keep the talktrack", "Сохранить запись"),
    ("Cancel", "Отмена"),
    ("Throw it away", "Выбросить"),
    ("Play", "Играть"),
    ("Space", "Пробел"),
    ("Version name", "Название версии"),
    ("Save version", "Сохранить версию"),
    (
        "Remember the board as it is now",
        "Запомнить доску как она есть сейчас",
    ),
    ("No saved versions yet", "Сохранённых версий пока нет"),
    (
        "Saved by {}. Look at the board as it was",
        "Сохранил(а) {}. Посмотреть доску такой, какой она была",
    ),
    ("Forget this version", "Забыть эту версию"),
    ("saved {} by {}", "сохранена {}, {}"),
    (
        "Changed and removed objects are outlined",
        "Изменённые и удалённые объекты обведены",
    ),
    ("Restore", "Восстановить"),
    (
        "Bring the board back to this version (Ctrl+Z undoes it)",
        "Вернуть доску к этой версии (Ctrl+Z отменяет)",
    ),
    ("The board is read-only", "Доска только для чтения"),
    ("Back to the board (Esc)", "Обратно к доске (Esc)"),
    // Presenting and focus.
    ("Exit", "Выход"),
    ("Esc", "Esc"),
    ("Focus mode", "Режим фокуса"),
    (
        "the rest of the board is out of reach",
        "остальная доска недоступна",
    ),
    ("Esc, Shift+F", "Esc, Shift+F"),
    ("Previous (←)", "Назад (←)"),
    ("Next (→, Space)", "Дальше (→, пробел)"),
    ("Laser", "Лазер"),
    ("Point with a red dot (L)", "Указывать красной точкой (L)"),
    // Export.
    ("Export", "Экспорт"),
    ("What", "Что"),
    ("Whole board", "Вся доска"),
    ("Selection", "Выделенное"),
    ("Frame", "Фрейм"),
    ("All frames, a page each", "Все фреймы, по странице"),
    (
        "PDF only, for boards with frames",
        "Только PDF, для досок с фреймами",
    ),
    ("Format", "Формат"),
    ("PNG", "PNG"),
    ("JPEG", "JPEG"),
    ("PDF", "PDF"),
    ("Markdown", "Markdown"),
    (
        "The text: frames as headings, notes as a list",
        "Текст: фреймы — заголовками, стикеры — списком",
    ),
    ("CSV", "CSV"),
    (
        "The first table, for spreadsheets",
        "Первая таблица, для Excel и Google Таблиц",
    ),
    ("Size", "Размер"),
    (
        "Pixels per board unit: 1× is the board at 100%",
        "Пикселей на единицу доски: 1× — как доска при масштабе 100%",
    ),
    ("Transparent background", "Прозрачный фон"),
    // Other windows.
    ("Diagram from text", "Диаграмма из текста"),
    (
        "A Mermaid flowchart: nodes like A[Box], B{Decision}, C((Circle)); edges like A --> B or A -->|label| B.",
        "Блок-схема Mermaid: узлы вида A[Блок], B{Решение}, C((Круг)); связи вида A --> B или A -->|подпись| B.",
    ),
    ("Insert", "Вставить"),
    ("Keyboard shortcuts", "Горячие клавиши"),
    ("Find on the board", "Поиск по доске"),
    ("No results", "Ничего не найдено"),
    ("{} of {}", "{} из {}"),
    ("Previous (Shift+Enter)", "Предыдущий (Shift+Enter)"),
    ("Next (Enter)", "Следующий (Enter)"),
    (
        "No tags yet: select objects and use Tags",
        "Тегов пока нет: выделите объекты и нажмите «Теги»",
    ),
    ("Change the color", "Сменить цвет"),
    (
        "Show objects with this tag",
        "Показать объекты с этим тегом",
    ),
    ("Remove the tag from the board", "Удалить тег с доски"),
    ("Colors", "Цвета"),
    ("Show everything", "Показать всё"),
    (
        "Show objects filled with this color",
        "Показать объекты с этой заливкой",
    ),
    // The text format bar.
    ("B", "Ж"),
    ("I", "К"),
    ("U", "Ч"),
    ("S", "З"),
    ("Bold (Ctrl+B)", "Жирный (Ctrl+B)"),
    ("Italic (Ctrl+I)", "Курсив (Ctrl+I)"),
    ("Underline (Ctrl+U)", "Подчёркнутый (Ctrl+U)"),
    ("Strikethrough (Ctrl+Shift+X)", "Зачёркнутый (Ctrl+Shift+X)"),
    ("Text color of the object", "Цвет текста объекта"),
    ("Text color", "Цвет текста"),
    ("None", "Нет"),
    // Shortcuts.
    ("Tools", "Инструменты"),
    ("Select", "Выделение"),
    (
        "Rectangle / ellipse / next shape",
        "Прямоугольник / эллипс / следующая фигура",
    ),
    ("Text", "Текст"),
    ("Sticky note", "Стикер"),
    ("Connector", "Коннектор"),
    ("Kanban board", "Канбан"),
    ("Markdown document", "Markdown-документ"),
    ("Tab / Enter", "Tab / Enter"),
    (
        "Mind map: add a subtopic / topic",
        "Майнд-карта: добавить подтему / тему",
    ),
    ("Edit the selected topic", "Править выбранную тему"),
    ("Pen / marker / eraser", "Ручка / маркер / ластик"),
    ("Full screen", "Во весь экран"),
    ("Clean board", "Чистая доска"),
    (
        "Hide all panels; a button in the corner brings them back",
        "Скрыть все панели; вернуть их — кнопкой в углу",
    ),
    ("Panels", "Панели"),
    ("Board color", "Цвет доски"),
    ("Graph of a function…", "График функции…"),
    ("Graph of a function", "График функции"),
    (
        "y = … of x, several separated by ;. For example: sin(x); x^2 - 3x + 1; 1/x; sqrt(x); |x|; 2^x; ln(x)",
        "y = … от x, несколько — через ;. Например: sin(x); x^2 - 3x + 1; 1/x; sqrt(x); |x|; 2^x; ln(x)",
    ),
    ("x from", "x от"),
    ("to", "до"),
    (
        "The range of x is not a pair of numbers.",
        "Диапазон x — не два числа.",
    ),
    ("Usual", "Обычный"),
    ("Menu", "Меню"),
    ("These controls", "Эта панель"),
    ("Hide the menu", "Скрыть меню"),
    ("Hide the tools", "Скрыть инструменты"),
    ("Hide these controls", "Скрыть эту панель"),
    ("Show the panels", "Показать панели"),
    ("Show the menu", "Показать меню"),
    ("Show the tools", "Показать инструменты"),
    ("Show the view controls", "Показать панель вида"),
    (
        "Clean board: hide the panels",
        "Чистая доска: скрыть панели",
    ),
    ("Shift (drawing)", "Shift (при рисовании)"),
    (
        "Straight line with the pen or marker",
        "Прямая линия ручкой или маркером",
    ),
    ("Editing", "Правка"),
    ("Enter, double click", "Enter, двойной щелчок"),
    ("Tab / Shift+Tab", "Tab / Shift+Tab"),
    (
        "Next sticky note, cell or card",
        "Следующий стикер, ячейка или карточка",
    ),
    (
        "Bold / italic / underline",
        "Жирный / курсив / подчёркнутый",
    ),
    ("Strikethrough", "Зачёркнутый"),
    ("Stop editing", "Закончить правку"),
    ("Copy / cut / paste", "Копировать / вырезать / вставить"),
    ("Ctrl+D, Alt+drag", "Ctrl+D, Alt+перетаскивание"),
    ("Arrows, Shift+arrows", "Стрелки, Shift+стрелки"),
    ("Nudge", "Сдвинуть"),
    (
        "Forward / backward (with Shift: front / back)",
        "Вперёд / назад (с Shift — на передний / задний план)",
    ),
    ("Group / ungroup", "Сгруппировать / разгруппировать"),
    ("Lock / unlock", "Заблокировать / разблокировать"),
    ("Hide / show", "Скрыть / показать"),
    ("Shift / Ctrl + click", "Shift / Ctrl + щелчок"),
    (
        "Add to or remove from the selection",
        "Добавить к выделению или убрать из него",
    ),
    (
        "Align left / right / top / bottom",
        "Выровнять влево / вправо / вверх / вниз",
    ),
    (
        "Center horizontally / vertically",
        "По центру по горизонтали / вертикали",
    ),
    (
        "Distribute horizontally / vertically",
        "Распределить по горизонтали / вертикали",
    ),
    ("Arrange in a grid", "Разложить сеткой"),
    ("Board", "Доска"),
    ("Undo / redo", "Отменить / повторить"),
    ("New / open / save", "Новая / открыть / сохранить"),
    ("Save as", "Сохранить как"),
    ("Export a picture or PDF", "Экспорт картинки или PDF"),
    ("Print (opens a PDF)", "Печать (открывает PDF)"),
    ("Wheel, Shift+wheel", "Колесо, Shift+колесо"),
    ("Scroll up and down, sideways", "Прокрутка вверх-вниз, вбок"),
    (
        "Space+drag, middle drag",
        "Пробел+перетаскивание, средняя кнопка",
    ),
    ("Pan", "Сдвиг"),
    ("Ctrl+wheel, Ctrl+= / -", "Ctrl+колесо, Ctrl+= / -"),
    ("Zoom", "Масштаб"),
    ("Zoom to 100%", "Масштаб 100%"),
    ("Back to the start", "Вернуться к началу"),
    ("G / Shift+G", "G / Shift+G"),
    ("Grid style / snap to grid", "Вид сетки / привязка к сетке"),
    ("This list", "Этот список"),
    (
        "Focus on the selection or the view",
        "Фокус на выделенном или на видимой области",
    ),
    ("Present the frames", "Показ фреймов"),
    ("L (presenting)", "L (при показе)"),
    ("Laser pointer", "Лазерная указка"),
    // Templates.
    ("SWOT analysis", "SWOT-анализ"),
    (
        "Strengths, weaknesses, opportunities and threats in four squares",
        "Сильные и слабые стороны, возможности и угрозы в четырёх квадратах",
    ),
    ("Retrospective", "Ретроспектива"),
    (
        "What went well, what to improve, and the actions to take",
        "Что получилось, что улучшить и какие шаги сделать",
    ),
    ("Kanban", "Канбан"),
    (
        "A board with To do, In progress and Done columns",
        "Доска с колонками «Сделать», «В работе» и «Готово»",
    ),
    ("Roadmap", "Дорожная карта"),
    (
        "Teams by quarter, in a table",
        "Команды по кварталам, в таблице",
    ),
    ("User story map", "Карта пользовательских историй"),
    (
        "Activities, the steps under them, and stories by release",
        "Действия, шаги под ними и истории по релизам",
    ),
    ("Customer journey", "Путь клиента"),
    (
        "Stages of the journey against actions, thoughts, feelings and more",
        "Этапы пути против действий, мыслей, чувств и прочего",
    ),
    ("Brainwriting 6-3-5", "Брейнрайтинг 6-3-5"),
    (
        "Six people, three ideas each, five rounds of building on them",
        "Шесть человек, по три идеи, пять раундов развития",
    ),
    (
        "A central topic with four branches to grow",
        "Главная тема с четырьмя ветками для развития",
    ),
    (
        "Start, steps, a decision and the end, joined by arrows",
        "Начало, шаги, решение и конец, соединённые стрелками",
    ),
    // Messages and names made by the app.
    ("Frame “{}”", "Фрейм «{}»"),
    ("An object on the board", "Объект на доске"),
    ("A deleted object", "Удалённый объект"),
    ("Version {}", "Версия {}"),
    (
        "Forget the version \"{}\"? This cannot be undone; the board itself stays as it is.",
        "Забыть версию «{}»? Это не отменить; сама доска не изменится.",
    ),
    ("Remove version", "Удалить версию"),
    (
        "Delete {} objects? Ctrl+Z brings them back.",
        "Удалить объекты ({})? Ctrl+Z вернёт их.",
    ),
    ("Layer {}", "Слой {}"),
    (
        "Images, PDF, tables, documents",
        "Картинки, PDF, таблицы, документы",
    ),
    ("{}: no cells", "{}: нет ячеек"),
    (
        "Some files could not be added",
        "Некоторые файлы не удалось добавить",
    ),
    ("Could not paste the image", "Не удалось вставить картинку"),
    ("Pasted table", "Вставленная таблица"),
    (
        "The table is too large: {}.",
        "Таблица слишком большая: {}.",
    ),
    (
        "only the first {} rows and {} columns were added",
        "добавлены только первые {} строк и {} столбцов",
    ),
    ("Central topic", "Главная тема"),
    ("Could not show the version", "Не удалось показать версию"),
    ("{} changed", "изменено: {}"),
    ("{} removed", "удалено: {}"),
    ("{} added", "добавлено: {}"),
    ("the same as now", "так же, как сейчас"),
    ("{} since", "{} с тех пор"),
    ("Print", "Печать"),
    ("There is nothing to print.", "Печатать нечего."),
    (
        "No app opens PDF files. The file is at {}",
        "Нет программы для PDF. Файл лежит в {}",
    ),
    ("Could not make the PDF: {}", "Не удалось сделать PDF: {}"),
    ("There is nothing to export.", "Экспортировать нечего."),
    ("{} selection", "{} — выделенное"),
    (
        "Could not save the file: {}",
        "Не удалось сохранить файл: {}",
    ),
    ("An object without text", "Объект без текста"),
    ("Talktrack {}", "Запись {}"),
    ("Could not open the link", "Не удалось открыть ссылку"),
    ("Could not open {}", "Не удалось открыть {}"),
    ("Save the board?", "Сохранить доску?"),
    (
        "This board has no file yet. Save it before closing it?",
        "У этой доски ещё нет файла. Сохранить её перед закрытием?",
    ),
    ("Could not save the board", "Не удалось сохранить доску"),
    ("Link target is gone", "Объекта по ссылке больше нет"),
    (
        "The object this links to has been deleted.",
        "Объект, на который ведёт ссылка, удалён.",
    ),
    ("Nothing recognized", "Ничего не распознано"),
    (
        "No text was found in the strokes.",
        "В штрихах не найден текст.",
    ),
    (
        "Handwriting recognition failed",
        "Рукописный текст не распознан",
    ),
    ("Recognized \"{}\": {}.", "Распознано «{}»: {}."),
    ("Could not calculate", "Не удалось вычислить"),
    ("just now", "только что"),
    ("{} min ago", "{} мин назад"),
    ("{} h ago", "{} ч назад"),
    ("{} d ago", "{} дн назад"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tr("…")` and `trf("…"` in the app's sources has a translation.
    #[test]
    fn every_text_shown_is_translated() {
        let sources = [
            include_str!("ui.rs"),
            include_str!("app.rs"),
            include_str!("export.rs"),
            include_str!("session.rs"),
        ];
        let table = russian();
        let mut missing = Vec::new();
        for source in sources {
            for call in ["tr(\"", "trf(\""] {
                for (i, _) in source.match_indices(call) {
                    // The literal up to its closing quote, escapes undone.
                    let mut text = String::new();
                    let mut chars = source[i + call.len()..].chars();
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => text.extend(chars.next()),
                            c => text.push(c),
                        }
                    }
                    if !table.contains_key(text.as_str()) && !missing.contains(&text) {
                        missing.push(text);
                    }
                }
            }
        }
        assert!(missing.is_empty(), "no translation for {missing:#?}");
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        assert_eq!(trf("{} of {} votes left", &[&2, &5]), "2 of 5 votes left");
        assert_eq!(Lang::from_code("ru-RU"), Some(Lang::Ru));
        assert_eq!(Lang::from_code("de"), None);
    }
}
