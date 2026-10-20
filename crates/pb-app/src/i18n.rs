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
    (
        "You can comment, react and vote here, but not change the board.",
        "Здесь можно комментировать, ставить реакции и голосовать, но не менять доску.",
    ),
    ("owner", "владелец"),
    ("can edit", "может править"),
    ("can comment", "может комментировать"),
    ("can look", "может смотреть"),
    ("Sign out", "Выйти"),
    ("Delete account…", "Удалить аккаунт…"),
    (
        "Deletes the account and the boards it owns from the server",
        "Удаляет аккаунт и его доски с сервера",
    ),
    (
        "None yet: boards you share while signed in, and those you are let in to, show here.",
        "Пока нет: здесь будут доски, которыми вы поделились после входа, и те, к которым вам открыли доступ.",
    ),
    ("Open this board", "Открыть эту доску"),
    ("Refresh", "Обновить"),
    (
        "The page it opens must show the code",
        "На открывшейся странице должен быть код",
    ),
    ("Sending the email…", "Отправляем письмо…"),
    (
        "Sign in to see your boards in one list and let people in by email.",
        "Войдите, чтобы видеть свои доски одним списком и открывать доступ по email.",
    ),
    ("Sign in", "Войти"),
    ("People let in by email", "Доступ по email"),
    (
        "They open the board from My boards after signing in with that address",
        "Они открывают доску из «Мои доски», войдя с этим адресом",
    ),
    ("Let in", "Открыть доступ"),
    (
        "They get an email about it",
        "Человеку придёт письмо об этом",
    ),
    ("New link", "Новая ссылка"),
    (
        "Makes a new link: the old one stops working",
        "Делает новую ссылку, старая перестаёт работать",
    ),
    ("Delete from the server…", "Удалить с сервера…"),
    (
        "Everyone loses the board; your copy here stays",
        "Доска пропадёт у всех; ваша копия здесь останется",
    ),
    ("Signed in as {}", "Вы вошли как {}"),
    ("My boards ({})", "Мои доски ({})"),
    (
        "A link to sign in was sent to {}. Open it in the email.",
        "Ссылка для входа отправлена на {}. Откройте её в письме.",
    ),
    (
        "Delete this board from the server? Everyone loses it; your copy here stays.",
        "Удалить эту доску с сервера? Она пропадёт у всех; ваша копия здесь останется.",
    ),
    ("Delete from the server", "Удалить с сервера"),
    ("Delete account", "Удалить аккаунт"),
    (
        "Delete your account? The boards it owns ({}) are deleted from the server for everyone. Copies of boards opened on this computer stay here: open the others from My boards first to keep them.",
        "Удалить ваш аккаунт? Его доски ({}) удалятся с сервера у всех. Копии досок, открытых на этом компьютере, останутся здесь: чтобы сохранить остальные, сначала откройте их из «Мои доски».",
    ),
    (
        "The link in the email has expired: sign in again.",
        "Ссылка в письме больше не работает: войдите заново.",
    ),
    (
        "This server has no accounts: share by links instead.",
        "На этом сервере нет аккаунтов: делитесь ссылками.",
    ),
    ("Signed out: sign in again.", "Вы вышли: войдите снова."),
    (
        "Only the board's owner can do that.",
        "Это может только владелец доски.",
    ),
    (
        "Too many attempts: try again later.",
        "Слишком много попыток: попробуйте позже.",
    ),
    ("That is not an email address.", "Это не адрес email."),
    (
        "The server could not send the email.",
        "Сервер не смог отправить письмо.",
    ),
    ("The server did not do it: {}", "Сервер этого не сделал: {}"),
    ("Plugins", "Плагины"),
    ("No plugin commands yet", "Команд плагинов пока нет"),
    ("Manage plugins…", "Управление плагинами…"),
    (
        "No plugins installed. A plugin is a folder with manifest.json and a script; see the user guide.",
        "Плагинов нет. Плагин — папка с manifest.json и скриптом; подробнее в руководстве.",
    ),
    ("On", "Включён"),
    ("development", "разработка"),
    (
        "Runs from its own folder and reloads when its files change",
        "Работает из своей папки и перезапускается, когда меняются её файлы",
    ),
    ("Asks for no permissions", "Не просит прав"),
    ("Not running", "Не запущен"),
    ("Install plugin…", "Установить плагин…"),
    (
        "Pick the plugin's manifest.json (in the browser: with its scripts)",
        "Выберите manifest.json плагина (в браузере — вместе с его скриптами)",
    ),
    ("Develop from a folder…", "Разрабатывать из папки…"),
    (
        "Runs a plugin from its folder and reloads it when you save its files",
        "Запускает плагин из его папки и перезапускает, когда вы сохраняете его файлы",
    ),
    (
        "Install plugins only from people you trust: a plugin can change your boards.",
        "Ставьте плагины только от тех, кому доверяете: плагин может менять ваши доски.",
    ),
    ("by {}", "автор: {}"),
    ("May: {}", "Может: {}"),
    ("Log ({})", "Журнал ({})"),
    ("Installed", "Установлен"),
    (
        "Could not install the plugin",
        "Не удалось установить плагин",
    ),
    (
        "PaintBoard plugin (manifest.json and scripts)",
        "Плагин PaintBoard (manifest.json и скрипты)",
    ),
    ("The plugin's manifest.json", "manifest.json плагина"),
    ("Remove plugin", "Удалить плагин"),
    ("Could not load the plugin", "Не удалось загрузить плагин"),
    (
        "Remove the plugin \"{}\" and what it keeps?",
        "Удалить плагин «{}» и то, что он хранит?",
    ),
    ("read the board", "читать доску"),
    ("change the board", "менять доску"),
    ("put text on the clipboard", "класть текст в буфер обмена"),
    ("fetch from {}", "загружать с {}"),
    ("Started", "Запущен"),
    (
        "Pick the plugin's manifest.json and its scripts.",
        "Выберите manifest.json плагина и его скрипты.",
    ),
    (
        "The plugin's script {} is missing.",
        "Нет скрипта плагина {}.",
    ),
    (
        "Reloaded: its files changed",
        "Перезапущен: его файлы изменились",
    ),
    (
        "Stopped: too many calls at once",
        "Остановлен: слишком много вызовов сразу",
    ),
    ("Install the plugin \"{}\" {}", "Установить плагин «{}» {}"),
    (" by {}", " (автор: {})"),
    (
        "It asks for no permissions: it can only show its own commands and panel.",
        "Он не просит прав: может только показывать свои команды и окно.",
    ),
    ("It will be able to:", "Он сможет:"),
    (
        "Install plugins only from people you trust.",
        "Ставьте плагины только от тех, кому доверяете.",
    ),
    ("Install plugin", "Установка плагина"),
    (
        "PaintBoard plugin (.pbplugin, or manifest.json and scripts)",
        "Плагин PaintBoard (.pbplugin или manifest.json со скриптами)",
    ),
    ("PaintBoard plugin", "Плагин PaintBoard"),
    ("Could not save the plugin", "Не удалось сохранить плагин"),
    ("Pack…", "Упаковать…"),
    (
        "Save it as one .pbplugin file to give to others",
        "Сохранить одним файлом .pbplugin, чтобы передать другим",
    ),
    (
        "Pick a .pbplugin file, or the plugin's manifest.json (in the browser: with its scripts)",
        "Выберите файл .pbplugin или manifest.json плагина (в браузере — вместе со скриптами)",
    ),
    ("Here now:", "Сейчас здесь:"),
    ("Link to comment", "Ссылка для комментариев"),
    (
        "Whoever has it can comment, react and vote, but not change the board",
        "У кого она есть, может комментировать, ставить реакции и голосовать, но не менять доску",
    ),
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
        "Pen draws, fingers move the board",
        "Перо рисует, пальцы двигают доску",
    ),
    (
        "On a touch screen with a pen: one finger pans, two zoom",
        "На сенсорном экране с пером: один палец двигает доску, два — масштабируют",
    ),
    ("Pen and touch", "Перо и касания"),
    ("Pen eraser end", "Перо обратной стороной"),
    (
        "Erase, whatever tool is picked",
        "Стереть, какой бы инструмент ни был выбран",
    ),
    ("Pen side button + stroke", "Боковая кнопка пера + штрих"),
    ("Erase", "Стереть"),
    ("Pen side button + tap", "Боковая кнопка пера + касание"),
    ("Right-click menu", "Контекстное меню"),
    ("One finger", "Один палец"),
    (
        "Pan (with fingers moving the board)",
        "Сдвинуть доску (когда пальцы двигают доску)",
    ),
    ("Two fingers", "Два пальца"),
    (
        "Zoom and pan (with fingers moving the board)",
        "Масштаб и сдвиг (когда пальцы двигают доску)",
    ),
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
    ("Everyone on the board sees it", "Его видят все на доске"),
    ("Sound when time is up", "Звук, когда время вышло"),
    (
        "A short chime on this computer",
        "Короткий сигнал на этом компьютере",
    ),
    ("The name is empty", "Пустое название"),
    (
        "There is already a board with this name",
        "Доска с таким названием уже есть",
    ),
    (
        "Could not rename the board",
        "Не удалось переименовать доску",
    ),
    ("Rename", "Переименовать"),
    ("OK", "ОК"),
    ("Download", "Скачать"),
    (
        "Open, rename, download or delete them",
        "Открыть, переименовать, скачать или удалить",
    ),
    ("+ New board", "+ Новая доска"),
    ("Open a file…", "Открыть файл…"),
    (
        "A .pboard file from this computer",
        "Файл .pboard с этого компьютера",
    ),
    ("No boards here yet", "Здесь пока нет досок"),
    ("Open now", "Открыта сейчас"),
    (
        "Your own time, like 7 or 2:30:",
        "Своё время, например 7 или 2:30:",
    ),
    ("min", "мин"),
    ("Start", "Запустить"),
    ("From 1 second to 24 hours", "От 1 секунды до 24 часов"),
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
    ("SVG", "SVG"),
    ("PDF (pictures)", "PDF (картинками)"),
    (
        "Shapes and text as lines, sharp at any size",
        "Фигуры и текст линиями, чёткие при любом размере",
    ),
    (
        "Each page a picture, as it looks on the board",
        "Каждая страница — картинка, как на доске",
    ),
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
    // The command palette.
    (
        "Command palette: any command, frame or text",
        "Командная палитра: любая команда, фрейм или текст",
    ),
    (
        "Type a command, a frame or text on the board",
        "Команда, фрейм или текст на доске",
    ),
    ("Nothing found", "Ничего не найдено"),
    ("Commands", "Команды"),
    (
        "Command palette: any command, frame or text (Ctrl+K)",
        "Командная палитра: любая команда, фрейм или текст (Ctrl+K)",
    ),
    ("On the board", "На доске"),
    ("Plugin", "Плагин"),
    ("Go to frame: {}", "Перейти к фрейму: {}"),
    ("Go to view {}", "Перейти к виду {}"),
    ("Save view as {}", "Сохранить вид как {}"),
    ("View {} saved", "Вид {} сохранён"),
    (
        "No view saved as {} yet: Ctrl+Shift+{} saves one",
        "Вид {} ещё не сохранён: сохранить — Ctrl+Shift+{}",
    ),
    ("View {} could not be saved", "Не удалось сохранить вид {}"),
    (
        "Save the view as 1…9 (on the board)",
        "Сохранить вид как 1…9 (в доске)",
    ),
    (
        "Go to the saved view 1…9",
        "Перейти к сохранённому виду 1…9",
    ),
    ("Tagged {}", "С тегом {}"),
    ("Undo", "Отменить"),
    ("Redo", "Повторить"),
    ("Record talktrack", "Начать запись"),
    ("Start voting", "Начать голосование"),
    ("Timer: 5 minutes", "Таймер: 5 минут"),
    ("Timer: 10 minutes", "Таймер: 10 минут"),
    ("Pause or resume the timer", "Пауза или продолжение таймера"),
    ("Close the timer", "Закрыть таймер"),
    ("Zoom in", "Увеличить"),
    ("Zoom out", "Уменьшить"),
    ("Zoom to fit", "Показать всё"),
    ("Grid style", "Вид сетки"),
    ("Pen", "Ручка"),
    ("Marker", "Маркер"),
    ("Eraser", "Ластик"),
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
    // Updates.
    ("PaintBoard {} is out", "Вышел PaintBoard {}"),
    (
        "Installing: PaintBoard will close and start again.",
        "Установка: PaintBoard закроется и запустится снова.",
    ),
    ("Downloading the update…", "Загрузка обновления…"),
    (
        "Could not download the update: {}",
        "Не удалось загрузить обновление: {}",
    ),
    ("Update", "Обновить"),
    ("What's new", "Что нового"),
    ("Later", "Позже"),
    ("Check for updates", "Проверить обновления"),
    (
        "Check for updates every day",
        "Проверять обновления каждый день",
    ),
    (
        "Could not check for updates: {}",
        "Не удалось проверить обновления: {}",
    ),
    (
        "You have the latest version ({}).",
        "У вас последняя версия ({}).",
    ),
    (
        "Could not start the installer",
        "Не удалось запустить установщик",
    ),
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
            include_str!("account.rs"),
            include_str!("plugins.rs"),
            include_str!("updates.rs"),
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
