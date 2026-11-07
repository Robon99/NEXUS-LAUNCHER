# Как опубликовать NEXUS Launcher на GitHub

Эта папка уже подготовлена для загрузки. Скриншоты не добавлены: репозиторий содержит исходный код и текстовые документы.

## 1. Создай пустой репозиторий

1. Войди на [GitHub](https://github.com/).
2. Нажми **+ → New repository**.
3. Repository name: `nexus-launcher`.
4. Description:

   `An independent Minecraft: Java Edition launcher prototype written in Rust and Slint.`

5. Выбери **Public** — так проверяющий сможет открыть исходники по ссылке.
6. Не ставь галочки Add a README, Add .gitignore и Choose a license: всё это уже есть в подготовленной папке.
7. Нажми **Create repository**.

Public — рекомендация для прозрачной проверки проекта, а не утверждение, что Minecraft обязательно принимает только публичные репозитории.

## 2. Загрузить файлы через сайт

1. Распакуй архив `NEXUS-Launcher-GitHub-Ready.zip`.
2. Открой созданный пустой репозиторий на GitHub.
3. Нажми **Add file → Upload files**.
4. Перетащи в окно загрузки ВСЕ файлы и папки из распакованного каталога проекта. Перетаскивай содержимое каталога, а не ZIP-архив.
5. Убедись, что на верхнем уровне репозитория видны `Cargo.toml`, `README.md`, `README_RU.md`, `LICENSE`, `PRIVACY.md`, `SECURITY.md`, `src/`, `ui/` и `docs/`.
6. Внизу в поле commit message напиши `Initial NEXUS Launcher source upload`.
7. Нажми **Commit changes**.

Если `.gitignore` не отображается в Проводнике, включи **Вид → Показать → Скрытые элементы**. На GitHub загружай отдельные файлы и папки, не помещай весь проект внутрь дополнительной папки `nexus-launcher`.

## 3. Данные репозитория

После создания нажми шестерёнку рядом с About (или **Edit repository details**) и укажи:

- Description: `An independent Minecraft: Java Edition launcher prototype written in Rust and Slint.`
- Website: можно оставить пустым или поставить URL самого репозитория.
- Topics: `minecraft`, `minecraft-launcher`, `rust`, `slint`, `modrinth`, `curseforge`.

Не используй логотип Minecraft как собственный логотип проекта и не называй NEXUS официальным продуктом Microsoft/Mojang.

## 4. Проверить публичную страницу

Открой `https://github.com/ТВОЙ-НИК/nexus-launcher` в приватном окне браузера. Проверь, что README отображается и папки `src`, `ui`, `docs` доступны без входа.

## 5. Подать Minecraft App ID Review

1. Создай приложение в Microsoft Entra по инструкции [`MICROSOFT_APP_ID_REVIEW.md`](MICROSOFT_APP_ID_REVIEW.md).
2. Скопируй Application (client) ID.
3. Открой официальный адрес: https://aka.ms/mce-reviewappid
4. Заполни поля фактическими данными. В качестве ссылки на код укажи `https://github.com/ТВОЙ-НИК/nexus-launcher`.
5. В поле описания/назначения адаптируй английский текст из `MICROSOFT_APP_ID_REVIEW.md`.
6. Отправь форму и сохрани подтверждение.

Точный набор полей формы может меняться. Не выдумывай данные, которых у тебя нет. Регистрация в Entra не равна одобрению Minecraft Services; дождись отдельного решения.

## 6. Перед публикацией не загружай секреты

В репозиторий нельзя добавлять:

- `NEXUS_MICROSOFT_CLIENT_ID` как локальную настройку, если не хочешь публиковать свой ID; технически это публичный идентификатор, но его лучше сообщать только там, где его требуют;
- `NEXUS_CURSEFORGE_API_KEY`;
- Microsoft access/refresh tokens;
- `.env`, `launcher.json`, персональные логи и содержимое папок игры.

`.gitignore` уже исключает типичные локальные секреты и build-файлы. Перед загрузкой всё равно проверь список файлов.

## 7. Команды Git вместо загрузки через сайт (необязательно)

Если установлен Git, можно загрузить проект командами PowerShell из папки, где лежат `Cargo.toml` и `README.md`:

```powershell
git init
git add .
git commit -m "Initial NEXUS Launcher source upload"
git branch -M main
git remote add origin https://github.com/ТВОЙ-НИК/nexus-launcher.git
git push -u origin main
```

Если Git попросит имя и email, настрой их командой `git config --global user.name "ТВОЁ ИМЯ"` и `git config --global user.email "ТВОЙ EMAIL"`. Не помещай пароль или токен в URL команд.

## Важное

Перед заявкой желательно выполнить на Windows:

```powershell
cargo check --all-targets
```

Если компиляция проходит, Cargo создаст `Cargo.lock`. Добавь его в репозиторий отдельным коммитом — для приложения это помогает закрепить версии зависимостей. Если компиляция ещё падает, репозиторий всё равно можно опубликовать как экспериментальный исходный проект, но в заявке и README надо честно указывать его статус. Не заявляй, что приложение полностью протестировано, если это ещё не так.
