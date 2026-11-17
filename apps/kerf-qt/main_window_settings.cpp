// MainWindow's Settings: a panel over the window, its pages named down its
// side. Each page is rows (ui/settings_kit.h): a setting's name and what it
// does, its control at the row's end, a hairline between one and the next.

#include "main_window.h"
#include "bridge.h"
#include "chat_panel.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "modal_scrim.h"
#include "project_sidebar.h"
#include "settings_kit.h"
#include "stage_shell.h"

#include <QApplication>
#include <QClipboard>
#include <QDesktopServices>
#include <QDir>
#include <QFileInfo>
#include <QFrame>
#include <QHBoxLayout>
#include <QHash>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QSettings>
#include <QStackedWidget>
#include <QTextEdit>
#include <QTimer>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>

#include <initializer_list>
#include <utility>

namespace kerf {

namespace {

// A field at a row's end is this wide; a key or a URL is long.
constexpr int kFieldWidth = 280;
constexpr int kNavWidth = 208;

// One of a few, each with the name the settings keep it under.
ui::SegmentedControl *choice(std::initializer_list<std::pair<const char *, const char *>> options, const QString &current = {}) {
    auto *control = new ui::SegmentedControl;
    int at = 0;
    for (const auto &[id, label] : options) {
        const int index = control->addSegment(QString::fromLatin1(label));
        control->setSegmentData(index, QString::fromLatin1(id));
        if (current == QLatin1String(id)) at = index;
    }
    control->setCurrentIndex(at, false);
    return control;
}

// Buttons as wide as what they say, from the start of their row.
QHBoxLayout *buttonRow(std::initializer_list<QWidget *> widgets) {
    auto *row = new QHBoxLayout;
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(tokens::space::s8);
    for (QWidget *widget : widgets) row->addWidget(widget, 0, Qt::AlignVCenter);
    row->addStretch(1);
    return row;
}

// Several controls at a row's end, as one.
QWidget *together(std::initializer_list<QWidget *> widgets) {
    auto *holder = new QWidget;
    auto *row = new QHBoxLayout(holder);
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(tokens::space::s8);
    for (QWidget *widget : widgets) row->addWidget(widget, 0, Qt::AlignVCenter);
    return holder;
}

QLineEdit *field(const QString &placeholder, bool secret = false) {
    auto *edit = new QLineEdit;
    edit->setPlaceholderText(placeholder);
    edit->setFixedWidth(kFieldWidth);
    if (secret) {
        edit->setEchoMode(QLineEdit::Password);
        edit->setClearButtonEnabled(true);
    }
    return edit;
}

// A field of JSON, in the mono face.
QTextEdit *jsonField(int height) {
    auto *edit = new QTextEdit;
    edit->setAcceptRichText(false);
    edit->setFont(tokens::font(tokens::timecode));
    edit->setFixedHeight(height);
    return edit;
}

// A quiet line under something wide: what was saved, what went wrong.
QLabel *note() {
    auto *label = new ui::WrapLabel;
    label->setProperty("quiet", true);
    label->setWordWrap(true);
    return label;
}

} // namespace

QString MainWindow::chosenIn(const ui::SegmentedControl *control) {
    return control && control->currentIndex() >= 0 ? control->segmentData(control->currentIndex()).toString() : QString();
}

void MainWindow::chooseIn(ui::SegmentedControl *control, const QString &id) {
    if (!control) return;
    for (int index = 0; index < control->count(); ++index)
        if (control->segmentData(index).toString() == id) {
            control->setCurrentIndex(index, false);
            return;
        }
}

void MainWindow::chooseAgentConnection(const QString &id) { chooseIn(m_connectionChoice, id); }

void MainWindow::chooseAgentProtocol(const QString &id) { chooseIn(m_protocolChoice, id); }

void MainWindow::buildPreferences() {
    // Settings opens over the window, as everything one goes into does: a
    // panel in the middle of the dimmed work, its pages named down its side.
    // Esc, its close, or a press on the dim gives the window back as it was.
    auto *scrim = new ModalScrim;
    scrim->setObjectName(QStringLiteral("settingsScrim"));
    scrim->setAccessibleName(QStringLiteral("Settings"));
    connect(scrim, &ModalScrim::dismissed, this, [this] { m_shell->setCovered(false); });
    m_preferences = scrim;
    auto *panel = new QWidget;
    panel->setObjectName(QStringLiteral("settingsPage"));
    panel->setAccessibleName(QStringLiteral("Settings"));
    panel->setAttribute(Qt::WA_StyledBackground);
    panel->setStyleSheet(QStringLiteral("QWidget#settingsPage { background: %1; border: 1px solid %4; border-radius: 12px; }"
                                        "QWidget#settingsNavColumn { background: transparent; border: 0; border-right: 1px solid %2; }"
                                        "QFrame#settingsRule { background: %2; border: 0; }"
                                        "QLabel#settingsTitle { color: %3; font-size: 15px; font-weight: 500; }")
                             .arg(tokens::color(tokens::chrome).name(), tokens::color(tokens::line).name(QColor::HexArgb),
                                  tokens::color(tokens::ink).name(), tokens::color(tokens::line2).name(QColor::HexArgb)));
    scrim->setPanel(panel);
    auto *pageLayout = new QVBoxLayout(panel);
    pageLayout->setContentsMargins(0, 0, 0, 0);
    pageLayout->setSpacing(0);
    // Its title, and the way out.
    auto *topRow = new QHBoxLayout;
    topRow->setContentsMargins(tokens::space::s16, tokens::space::s12, tokens::space::s12, tokens::space::s12);
    auto *settingsTitle = new QLabel(QStringLiteral("Settings"));
    settingsTitle->setObjectName(QStringLiteral("settingsTitle"));
    topRow->addWidget(settingsTitle);
    topRow->addStretch(1);
    auto *close = new ui::IconButton(QStringLiteral("g-close"), ui::IconButton::Size::S28);
    close->setObjectName(QStringLiteral("settingsClose"));
    close->setCheckable(false);
    close->setAccessibleName(QStringLiteral("Close Settings"));
    close->setTip(QStringLiteral("Close"), QStringLiteral("Esc"));
    connect(close, &QAbstractButton::clicked, this, [this] { m_shell->setCovered(false); });
    topRow->addWidget(close);
    pageLayout->addLayout(topRow);
    auto *rule = new QFrame;
    rule->setObjectName(QStringLiteral("settingsRule"));
    rule->setFixedHeight(1);
    pageLayout->addWidget(rule);

    m_settingsBody = new QWidget;
    m_settingsBody->setObjectName(QStringLiteral("settingsBody"));
    auto *body = new QHBoxLayout(m_settingsBody);
    body->setContentsMargins(0, 0, 0, 0);
    body->setSpacing(0);
    // The pages, named down the side, each with its glyph.
    auto *navColumn = new QWidget;
    navColumn->setObjectName(QStringLiteral("settingsNavColumn"));
    navColumn->setAttribute(Qt::WA_StyledBackground);
    navColumn->setFixedWidth(kNavWidth);
    auto *nav = new QVBoxLayout(navColumn);
    nav->setContentsMargins(tokens::space::s8, tokens::space::s12, tokens::space::s8, tokens::space::s12);
    nav->setSpacing(tokens::space::s2);
    m_settingsPages = new QStackedWidget;
    m_settingsPages->setObjectName(QStringLiteral("settingsPages"));
    static const QHash<QString, QString> glyphs{
        {QStringLiteral("account"), QStringLiteral("g-user")},   {QStringLiteral("ai"), QStringLiteral("g-ask")},
        {QStringLiteral("models"), QStringLiteral("g-fx")},      {QStringLiteral("appearance"), QStringLiteral("g-eye")},
        {QStringLiteral("general"), QStringLiteral("g-settings")}, {QStringLiteral("advanced"), QStringLiteral("g-branch")}};
    const auto addPage = [this, nav](const QString &id, const QString &name) {
        auto *row = new SidebarRow(glyphs.value(id), name);
        row->setObjectName(QStringLiteral("settingsNav.%1").arg(id));
        nav->addWidget(row);
        auto *page = new ui::SettingsPage(name);
        page->setObjectName(QStringLiteral("settingsScroll.%1").arg(id));
        const int index = m_settingsPages->addWidget(page);
        m_settingsIds << id;
        m_settingsNav << row;
        connect(row, &QAbstractButton::clicked, this, [this, id] { showSettings(id); });
        row->setCurrent(index == 0);
        return page;
    };
    ui::SettingsPage *accountPage = addPage(QStringLiteral("account"), QStringLiteral("Account"));
    ui::SettingsPage *chatPage = addPage(QStringLiteral("ai"), QStringLiteral("Chat"));
    ui::SettingsPage *modelsPage = addPage(QStringLiteral("models"), QStringLiteral("Models"));
    ui::SettingsPage *appearancePage = addPage(QStringLiteral("appearance"), QStringLiteral("Appearance"));
    ui::SettingsPage *generalPage = addPage(QStringLiteral("general"), QStringLiteral("General"));
    ui::SettingsPage *mcpPage = addPage(QStringLiteral("advanced"), QStringLiteral("MCP"));
    nav->addStretch(1);
    body->addWidget(navColumn);
    body->addWidget(m_settingsPages, 1);
    pageLayout->addWidget(m_settingsBody, 1);

    // Account: who is signed in to Kerf Cloud, and the way in or out.
    m_cloudToken = field(QStringLiteral("Sign-in token"), true);
    m_cloudToken->setObjectName(QStringLiteral("cloudToken"));
    m_cloudToken->setFixedWidth(220);
    m_cloudToken->setAccessibleName(QStringLiteral("Kerf Cloud sign-in token"));
    m_cloudSignIn = new ui::Button(QStringLiteral("Sign in"), ui::Button::Kind::Primary);
    m_cloudSignIn->setObjectName(QStringLiteral("cloudSignIn"));
    const auto signIn = [this] {
        const QString token = m_cloudToken->text().trimmed();
        if (token.isEmpty()) return;
        if (sendCloud(QJsonObject{{QStringLiteral("type"), QStringLiteral("cloud_sign_in")}, {QStringLiteral("token"), token}}))
            m_cloudToken->clear();
    };
    connect(m_cloudSignIn, &QAbstractButton::clicked, this, signIn);
    connect(m_cloudToken, &QLineEdit::returnPressed, this, signIn);
    m_cloudSignOut = new ui::Button(QStringLiteral("Sign out"));
    m_cloudSignOut->setObjectName(QStringLiteral("cloudSignOut"));
    connect(m_cloudSignOut, &QAbstractButton::clicked, this,
            [this] { sendCloud(QJsonObject{{QStringLiteral("type"), QStringLiteral("cloud_sign_out")}}); });
    m_accountRow = accountPage->addGroup()->addRow(QStringLiteral("Kerf Cloud"), QString(),
                                                   together({m_cloudToken, m_cloudSignIn, m_cloudSignOut}));
    m_accountRow->setObjectName(QStringLiteral("accountStatus"));

    // Chat: what the conversation runs on. Kerf Cloud, the hosted model, comes
    // first; a key of one's own shows its fields under the choice. The models
    // that make shots are the Models page's, and MCP its own.
    ui::SettingsGroup *chatGroup = chatPage->addGroup();
    m_connectionChoice = choice({{"cloud", "Kerf Cloud"}, {"own", "My API key"}, {"claude-code", "Claude Code"}});
    m_connectionChoice->setObjectName(QStringLiteral("agentConnection"));
    // None until the settings are read, so that reading them chooses one.
    m_connectionChoice->setCurrentIndex(-1, false);
    m_connectionChoice->setSegmentTip(2, QStringLiteral("Your installed Claude Code, on your Claude plan"));
    m_chatModelRow = chatGroup->addRow(QStringLiteral("Chat model"), QString(), m_connectionChoice);
    m_chatModelRow->setObjectName(QStringLiteral("cloudStatus"));

    // A key of one's own: one protocol in use, and that one's fields.
    ui::SettingsGroup *ownKey = chatGroup->addRows();
    ownKey->setObjectName(QStringLiteral("ownKeySettings"));
    m_ownKeySettings = ownKey;
    m_protocolChoice = choice({{"anthropic", "Anthropic"}, {"openai", "OpenAI-compatible"}});
    m_protocolChoice->setObjectName(QStringLiteral("agentProtocol"));
    ownKey->addRow(QStringLiteral("Protocol"), QStringLiteral("OpenAI-compatible is for any service that speaks the OpenAI API"),
                   m_protocolChoice);
    const QString keyNote = QStringLiteral("Kept in your system keychain");
    const QString modelNote = QStringLiteral("Empty for the service's default");
    ui::SettingsGroup *anthropicRows = ownKey->addRows();
    anthropicRows->setObjectName(QStringLiteral("providerRows.anthropic"));
    m_anthropicKey = field(QStringLiteral("sk-ant-…"), true);
    m_anthropicBaseUrl = field(QStringLiteral("https://…/v1"));
    m_anthropicBaseUrl->setText(QStringLiteral("https://api.anthropic.com/v1"));
    m_anthropicModel = field(QStringLiteral("Default"));
    anthropicRows->addRow(QStringLiteral("API key"), keyNote, m_anthropicKey);
    anthropicRows->addRow(QStringLiteral("Base URL"), QString(), m_anthropicBaseUrl);
    anthropicRows->addRow(QStringLiteral("Model"), modelNote, m_anthropicModel);
    ui::SettingsGroup *openaiRows = ownKey->addRows();
    openaiRows->setObjectName(QStringLiteral("providerRows.openai"));
    m_openaiKey = field(QStringLiteral("sk-…"), true);
    m_openaiBaseUrl = field(QStringLiteral("https://…/v1"));
    m_openaiBaseUrl->setText(QStringLiteral("https://api.openai.com/v1"));
    m_openaiModel = field(QStringLiteral("Default"));
    m_openaiMode = choice({{"chat", "Chat Completions"}, {"responses", "Responses"}});
    m_openaiMode->setObjectName(QStringLiteral("agentOpenaiMode"));
    openaiRows->addRow(QStringLiteral("API key"), keyNote, m_openaiKey);
    openaiRows->addRow(QStringLiteral("Base URL"), QStringLiteral("Must serve /chat/completions or /responses"), m_openaiBaseUrl);
    openaiRows->addRow(QStringLiteral("Model"), modelNote, m_openaiModel);
    openaiRows->addRow(QStringLiteral("API shape"), QString(), m_openaiMode);
    const auto showProtocol = [this, anthropicRows, openaiRows] {
        const bool openai = activeProviderId() == QStringLiteral("openai");
        anthropicRows->setVisible(!openai);
        openaiRows->setVisible(openai);
    };

    // How far Kerf goes without asking: the same choice as the composer's chip.
    m_permissionChoice = new ui::SegmentedControl;
    m_permissionChoice->setObjectName(QStringLiteral("settingsPermissions"));
    m_permissionChoice->setAccessibleName(QStringLiteral("What Kerf does without asking"));
    for (const PermissionMode mode : {PermissionMode::Ask, PermissionMode::AcceptEdits, PermissionMode::Auto}) {
        const int index = m_permissionChoice->addSegment(permission::label(mode), permission::summary(mode));
        m_permissionChoice->setSegmentData(index, permission::name(mode));
    }
    m_permissionRow = chatGroup->addRow(QStringLiteral("Permissions"), QString(), m_permissionChoice);
    const auto describePermission = [this](PermissionMode mode) {
        QString said;
        switch (mode) {
        case PermissionMode::Ask:
            said = QStringLiteral("Every edit is a proposal you accept or reject");
            break;
        case PermissionMode::AcceptEdits:
            said = QStringLiteral("Edits apply as Kerf makes them, and Undo takes each back. Shots still ask");
            break;
        case PermissionMode::Auto:
            said = QStringLiteral("Edits apply, and Kerf makes the shots it asks for, five before it asks again. "
                                  "Shots can cost time and money");
            break;
        }
        m_permissionRow->setDescription(said);
    };
    {
        const PermissionMode mode = m_chat->permissionMode();
        m_permissionChoice->setCurrentIndex(int(mode), false);
        describePermission(mode);
    }
    connect(m_permissionChoice, &ui::SegmentedControl::activated, this, [this, describePermission](int index) {
        const PermissionMode mode = permission::named(m_permissionChoice->segmentData(index).toString());
        describePermission(mode);
        if (mode == m_chat->permissionMode()) return;
        m_chat->setPermissionMode(mode);
        QSettings().setValue(QStringLiteral("agent/permissions"), permission::name(mode));
    });
    // Chosen at the composer, it shows here too.
    connect(m_chat, &ChatPanel::permissionModeChosen, this, [this, describePermission](PermissionMode mode) {
        m_permissionChoice->setCurrentIndex(int(mode), false);
        describePermission(mode);
    });

    // Appearance: the faces text is set in. The interface's, and what is said
    // in the conversation, which may have a face and a size of its own.
    const FaceChoice faces = loadFaceChoice();
    ui::SettingsGroup *faceGroup = appearancePage->addGroup();
    m_interfaceFace = choice({{"inter", "Inter"}, {"pretendard", "Pretendard"}, {"system", "System"}}, faces.ui);
    m_interfaceFace->setObjectName(QStringLiteral("settingsInterfaceFace"));
    faceGroup->addRow(QStringLiteral("Interface font"), QStringLiteral("With Inter, Korean is set in Pretendard"), m_interfaceFace);
    m_readingFace = choice({{"same", "Same as interface"}, {"serif", "Serif"}}, faces.reading);
    m_readingFace->setObjectName(QStringLiteral("settingsReadingFace"));
    faceGroup->addRow(QStringLiteral("Conversation font"), QStringLiteral("For what you ask and what Kerf answers"), m_readingFace);
    m_readingSize = choice({{"small", "Small"}, {"default", "Default"}, {"large", "Large"}}, faces.size);
    m_readingSize->setObjectName(QStringLiteral("settingsReadingSize"));
    ui::SettingsRow *sizeRow = faceGroup->addRow(QStringLiteral("Conversation text size"), QString(), m_readingSize);
    // What a request and an answer will read like.
    m_readingSample = new ui::WrapLabel;
    m_readingSample->setObjectName(QStringLiteral("settingsReadingSample"));
    m_readingSample->setAccessibleName(QStringLiteral("Sample"));
    m_readingSample->setWordWrap(true);
    m_readingSample->setTextFormat(Qt::PlainText);
    m_readingSample->setText(QStringLiteral("Make a 30-second cut from the best takes, and open on the wide shot.\n"
                                            "가장 좋은 테이크로 30초 컷을 만들고, 와이드 숏으로 시작해 줘."));
    m_readingSample->setFont(tokens::font(tokens::reading));
    m_readingSample->setStyleSheet(QStringLiteral("QLabel#settingsReadingSample { color: %1; background: %2; border: 1px solid %3; "
                                                  "border-radius: 8px; padding: 12px; }")
                                       .arg(tokens::color(tokens::ink).name(), tokens::color(tokens::card).name(),
                                            tokens::color(tokens::line).name(QColor::HexArgb)));
    sizeRow->addBelow(m_readingSample);
    const auto applyFaces = [this] {
        FaceChoice chosen;
        chosen.ui = chosenIn(m_interfaceFace);
        chosen.reading = chosenIn(m_readingFace);
        chosen.size = chosenIn(m_readingSize);
        saveFaceChoice(chosen);
        setEditorFaces(chosen);
        m_readingSample->setFont(tokens::font(tokens::reading));
    };
    for (ui::SegmentedControl *control : {m_interfaceFace, m_readingFace, m_readingSize})
        connect(control, &ui::SegmentedControl::activated, this, applyFaces);

    // General. Whether Kerf asks GitHub once a day for a newer release (update_notice.h).
    auto *checkForUpdates = new ui::Switch;
    checkForUpdates->setObjectName(QStringLiteral("checkForUpdates"));
    checkForUpdates->setChecked(QSettings().value(QStringLiteral("updates/check"), true).toBool());
    connect(checkForUpdates, &QAbstractButton::toggled, this, [](bool on) { QSettings().setValue(QStringLiteral("updates/check"), on); });
    generalPage->addGroup()->addRow(QStringLiteral("Update notices"),
                                    QStringLiteral("Checks GitHub once a day. Never downloads or installs"), checkForUpdates);

    const auto onConnectionChanged = [this] {
        QSettings().setValue(QStringLiteral("agent/connection"), chosenIn(m_connectionChoice));
        m_ownKeySettings->setVisible(chosenIn(m_connectionChoice) == QStringLiteral("own"));
        updateAgentModelChoices();
        updateCloudStatus();
    };
    connect(m_connectionChoice, &ui::SegmentedControl::currentChanged, this, onConnectionChanged);

    // MCP, in both directions: the tools Kerf's chat may call, and Kerf as a
    // server another client calls.
    ui::SettingsGroup *inbound = mcpPage->addGroup(QStringLiteral("Tools in Kerf chat"));
    m_advancedSettings = inbound;
    inbound->setObjectName(QStringLiteral("advancedSettings"));
    m_mcpEnabled = new ui::Switch;
    m_mcpEnabled->setObjectName(QStringLiteral("inboundMcpEnabled"));
    const bool environmentMcpEnabled = qEnvironmentVariable("KERF_ENABLE_MCP") == QStringLiteral("1");
    m_mcpEnabled->setChecked(QSettings().value(QStringLiteral("agent/mcpEnabled"), environmentMcpEnabled).toBool());
    inbound->addRow(QStringLiteral("External MCP tools"), QStringLiteral("Lets Kerf chat call local MCP servers, over stdio"),
                    m_mcpEnabled);
    const QString inboundPath = QDir(m_projectDirectory).absoluteFilePath(QStringLiteral(".kerf/mcp.json"));
    ui::SettingsRow *configRow = inbound->addRow(
        QStringLiteral("Servers"), QStringLiteral("This project's .kerf/mcp.json, then ~/.kerf/mcp.json. It may hold credentials"));
    configRow->setToolTip(QDir::toNativeSeparators(inboundPath));
    m_inboundMcpJson = jsonField(160);
    m_inboundMcpJson->setObjectName(QStringLiteral("inboundMcpConfigJson"));
    m_inboundMcpJson->setAccessibleName(QStringLiteral("MCP servers, as JSON"));
    m_inboundMcpJson->setPlainText(QStringLiteral("{\n  \"mcpServers\": {}\n}\n"));
    m_inboundMcpJson->setPlaceholderText(QStringLiteral("{\"mcpServers\": {\"example\": {\"command\": \"/path/to/server\", \"args\": [], \"env\": {}}}}"));
    configRow->addBelow(m_inboundMcpJson);
    auto *saveInbound = new ui::Button(QStringLiteral("Save and apply"));
    saveInbound->setObjectName(QStringLiteral("saveInboundMcpConfig"));
    connect(saveInbound, &QAbstractButton::clicked, this, &MainWindow::saveInboundMcpConfig);
    auto *loadProject = new ui::Button(QStringLiteral("Load from project's Cursor"));
    loadProject->setObjectName(QStringLiteral("loadProjectCursorMcp"));
    const QString projectCursor = QDir(m_projectDirectory).absoluteFilePath(QStringLiteral(".cursor/mcp.json"));
    loadProject->setTip(QDir::toNativeSeparators(projectCursor));
    connect(loadProject, &QAbstractButton::clicked, this, [this, projectCursor] { loadInboundMcpConfig(projectCursor, true); });
    auto *loadHome = new ui::Button(QStringLiteral("Load from home Cursor"));
    loadHome->setObjectName(QStringLiteral("loadHomeCursorMcp"));
    const QString homeCursor = QDir::home().filePath(QStringLiteral(".cursor/mcp.json"));
    loadHome->setTip(QDir::toNativeSeparators(homeCursor));
    connect(loadHome, &QAbstractButton::clicked, this, [this, homeCursor] { loadInboundMcpConfig(homeCursor, true); });
    configRow->addBelow(buttonRow({saveInbound, loadProject, loadHome}));
    m_inboundMcpStatus = note();
    m_inboundMcpStatus->setObjectName(QStringLiteral("inboundMcpStatus"));
    m_inboundMcpStatus->setText(QStringLiteral("No project config yet"));
    configRow->addBelow(m_inboundMcpStatus);
    if (QFileInfo::exists(inboundPath)) loadInboundMcpConfig(inboundPath, false);
    connect(m_mcpEnabled, &QAbstractButton::toggled, this, [this](bool enabled) {
        QSettings settings;
        settings.setValue(QStringLiteral("agent/mcpEnabled"), enabled);
        settings.sync();
        m_inboundMcpStatus->setText(enabled ? QStringLiteral("On from the next request") : QStringLiteral("Off from the next request"));
    });

    ui::SettingsGroup *outbound = mcpPage->addGroup(QStringLiteral("Kerf as an MCP server"));
    outbound->setObjectName(QStringLiteral("mcpSection"));
    const QString projectPath = QDir(m_projectDirectory).absolutePath();
    const QJsonObject config{{QStringLiteral("mcpServers"), QJsonObject{
        {QStringLiteral("kerf"), QJsonObject{
            {QStringLiteral("command"), mcpExecutable(projectPath)},
            {QStringLiteral("args"), QJsonArray{QStringLiteral("--dir"), projectPath}}
        }}}}};
    auto *copyMcp = new ui::Button(QStringLiteral("Copy config"));
    copyMcp->setObjectName(QStringLiteral("copyMcpConfig"));
    ui::SettingsRow *serverRow = outbound->addRow(
        QStringLiteral("Use Kerf from Cursor, Claude or Codex"), QStringLiteral("Add this to the client's MCP settings"), copyMcp);
    auto *mcpJson = jsonField(132);
    mcpJson->setObjectName(QStringLiteral("mcpConfigJson"));
    mcpJson->setAccessibleName(QStringLiteral("Kerf's MCP config, as JSON"));
    mcpJson->setReadOnly(true);
    mcpJson->setPlainText(QString::fromUtf8(QJsonDocument(config).toJson(QJsonDocument::Indented)));
    serverRow->addBelow(mcpJson);
    connect(copyMcp, &QAbstractButton::clicked, this, [copyMcp, mcpJson] {
        QApplication::clipboard()->setText(mcpJson->toPlainText());
        copyMcp->setText(QStringLiteral("Copied"));
        copyMcp->updateGeometry();
        QTimer::singleShot(2000, copyMcp, [copyMcp] {
            copyMcp->setText(QStringLiteral("Copy config"));
            copyMcp->updateGeometry();
        });
    });

    connect(m_protocolChoice, &ui::SegmentedControl::currentChanged, this, showProtocol);
    loadAgentSettings();
    m_ownKeySettings->setVisible(chosenIn(m_connectionChoice) == QStringLiteral("own"));
    showProtocol();

    // Models: the models that make shots, which are the person's own, and
    // those that run inside Kerf. Kerf runs none of the first to learn of
    // them: the file says what each takes.
    ui::SettingsGroup *modelsGroup = modelsPage->addGroup();
    m_modelsOpen = new ui::Button(QStringLiteral("Open models file"));
    m_modelsOpen->setObjectName(QStringLiteral("settingsModelsOpen"));
    connect(m_modelsOpen, &QAbstractButton::clicked, this, [this] {
        const QFileInfo file(m_models.value(QStringLiteral("file")).toString());
        if (file.filePath().isEmpty()) return;
        // Not there yet: its folder, to put it in.
        QDir().mkpath(file.absolutePath());
        QDesktopServices::openUrl(QUrl::fromLocalFile(file.exists() ? file.absoluteFilePath() : file.absolutePath()));
    });
    auto *modelsReload = new ui::Button(QStringLiteral("Reload"));
    modelsReload->setObjectName(QStringLiteral("settingsModelsReload"));
    connect(modelsReload, &QAbstractButton::clicked, this, [this] {
        if (m_bridge) sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("models")}});
    });
    ui::SettingsRow *shotModels = modelsGroup->addRow(
        QStringLiteral("Shot models"), QStringLiteral("Video and image models your recipes can name, from your models file"),
        together({m_modelsOpen, modelsReload}));
    shotModels->setObjectName(QStringLiteral("makingModels"));
    m_modelsList = new ui::WrapLabel;
    m_modelsList->setObjectName(QStringLiteral("settingsModels"));
    m_modelsList->setWordWrap(true);
    m_modelsList->setTextFormat(Qt::RichText);
    m_modelsList->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_modelsList->setStyleSheet(QStringLiteral("QLabel#settingsModels { color: %1; font-size: 14px; }").arg(tokens::color(tokens::ink).name()));
    shotModels->addBelow(m_modelsList);
    m_modelsFile = note();
    m_modelsFile->setObjectName(QStringLiteral("settingsModelsFile"));
    m_modelsFile->setFont(tokens::font(tokens::monoSmall));
    m_modelsFile->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_modelsFile->setStyleSheet(QStringLiteral("QLabel#settingsModelsFile { color: %1; }").arg(tokens::color(tokens::ink3).name()));
    shotModels->addBelow(m_modelsFile);
    modelsGroup->addRow(QStringLiteral("File format"),
                        QStringLiteral("Each entry names a program to run with {request} and, under takes, the size, rate, "
                                       "longest shot and files (guide, first, last, ref) its model accepts"));
    ui::SettingsRow *builtIn = modelsGroup->addRow(
        QStringLiteral("Built in"), QStringLiteral("Scene cuts, speech, tracking and cut-outs run on this computer, with no setup"));
    builtIn->setObjectName(QStringLiteral("builtInModels"));
    applyModels(QJsonObject{});

    // What is typed is kept when its field is left; a field still being typed
    // in when Settings closes is kept then.
    const auto onProtocolChanged = [this] {
        updateAgentModelChoices();
        saveAgentSettings();
    };
    connect(m_protocolChoice, &ui::SegmentedControl::currentChanged, this, onProtocolChanged);
    connect(m_openaiMode, &ui::SegmentedControl::currentChanged, this, &MainWindow::saveAgentSettings);
    for (QLineEdit *edit : {m_anthropicKey, m_anthropicBaseUrl, m_anthropicModel, m_openaiKey, m_openaiBaseUrl, m_openaiModel}) {
        connect(edit, &QLineEdit::editingFinished, this, [this] {
            if (m_agentFieldsEdited) saveAgentSettings();
        });
        connect(edit, &QLineEdit::textEdited, this, [this] { m_agentFieldsEdited = true; });
    }
    connect(m_shell, &StageShell::coveredChanged, this, [this](bool covered) {
        if (!covered && m_agentFieldsEdited) saveAgentSettings();
        // The toolbar is not the shell's to cover: it dims with the rest, so
        // the window is one room behind the panel, not a lit strip over a dim one.
        if (!m_toolbar) return;
        auto *dim = m_toolbar->findChild<ModalDim *>(QStringLiteral("modalDim"), Qt::FindDirectChildrenOnly);
        if (!dim && covered) {
            dim = new ModalDim(m_toolbar);
            connect(dim, &ModalDim::dismissed, this, [this] { m_shell->setCovered(false); });
        }
        if (!dim) return;
        dim->setVisible(covered);
        if (covered) dim->raise();
    });
}

} // namespace kerf
