#include "project_sidebar.h"
#include "chat_panel.h"

#include "change_reel.h"
#include "components.h"
#include "composer.h"
#include "design_tokens.h"
#include "editor_theme.h"
#include "icons.h"
#include "proposal_card.h"
#include "stage_motion.h"
#include "ui/chat_messages.h"

#include <QFileInfo>
#include <QDir>
#include <QCoreApplication>
#include <QDataStream>
#include <QFontMetricsF>
#include <QFrame>
#include <QHBoxLayout>
#include <QJsonDocument>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QPainter>
#include <QPushButton>
#include <QScrollBar>
#include <QTextEdit>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>
#include <memory>
#include <utility>

namespace kerf {

using namespace tokens;

namespace {

QLabel *makeLabel(const TypeStyle &style, QRgb ink, QWidget *parent = nullptr) {
    auto *label = new QLabel(parent);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
    return label;
}

QJsonObject findClip(const QJsonObject &timeline, const QString &clipId) {
    for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray())
        for (const QJsonValue &clip : track.toObject().value(QStringLiteral("clips")).toArray())
            if (clip.toObject().value(QStringLiteral("id")).toString() == clipId) return clip.toObject();
    return {};
}

// A colour effect: what a proposal card says in a line of its own.
bool isColourEffect(const QString &fx) {
    const QString id = fx.toLower();
    return id.startsWith(QLatin1String("color.")) || id.startsWith(QLatin1String("colour.")) || id.contains(QLatin1String("hue")) ||
           id.contains(QLatin1String("grade")) || id.contains(QLatin1String("lut")) || id.contains(QLatin1String("look")) ||
           id.contains(QLatin1String("curve")) || id.contains(QLatin1String("balance")) || id.contains(QLatin1String("saturation"));
}

// "color.huehue" → "Huehue", "color.white_balance" → "White balance".
QString effectLabel(const QString &fx) {
    QString name = fx.section(QLatin1Char('.'), -1);
    name.replace(QLatin1Char('_'), QLatin1Char(' '));
    return name.isEmpty() ? fx : name.left(1).toUpper() + name.mid(1);
}

// A parameter's value as the card says it: "−9", "0.5"; empty for keyframes.
QString paramValue(const QJsonValue &value) {
    if (value.isDouble()) {
        const double number = value.toDouble();
        const QString magnitude = QString::number(std::abs(number), 'g', 4);
        return number < 0 ? QStringLiteral("−") + magnitude : magnitude;
    }
    if (value.isBool()) return value.toBool() ? QStringLiteral("on") : QStringLiteral("off");
    if (value.isString()) return value.toString();
    return {};
}

// The static parameters only (keyframed ones cannot be previewed as one value).
QJsonObject staticParams(const QJsonObject &params) {
    QJsonObject out;
    for (auto it = params.begin(); it != params.end(); ++it)
        if (!it.value().isArray() && !it.value().isObject()) out.insert(it.key(), it.value());
    return out;
}

// What Try asking offers when Kerf has said nothing of its own yet.
const QStringList &firstRequests() {
    static const QStringList requests{QStringLiteral("Make a 30-second cut from the best takes"),
                                      QStringLiteral("Tighten the pauses in the interview"),
                                      QStringLiteral("Add captions to the whole sequence")};
    return requests;
}

} // namespace

// ------------------------------------------------------------------ ChatIconButton

ChatIconButton::ChatIconButton(Glyph glyph, int diameter, QWidget *parent)
    : QAbstractButton(parent), m_glyph(glyph), m_diameter(diameter) {
    setFixedSize(diameter, diameter);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
}

QSize ChatIconButton::sizeHint() const { return QSize(m_diameter, m_diameter); }

void ChatIconButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const bool hover = isEnabled() && (underMouse() || isDown());
    if (hover) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    }
    const QColor ink = !isEnabled() ? color(dis) : hover ? color(tokens::ink) : color(ink2);
    const QString icon = m_glyph == Glyph::Close ? QStringLiteral("g-close") : QStringLiteral("g-more");
    icons::paint(p, icon, QRectF(rect()), ink, m_diameter <= 20 ? 14 : 16);
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------ ChatPanel

ChatPanel::ChatPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("chatPanel"));
    setAttribute(Qt::WA_StyledBackground, false);
    setAccessibleName(QStringLiteral("Conversation"));

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(buildHeader());

    m_list = new ui::FadeScrollArea;
    m_list->setObjectName(QStringLiteral("chatScroll"));
    m_list->setWidgetResizable(true);
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setFadeColor(color(tokens::chrome));
    m_list->setStyleSheet(QStringLiteral("QScrollArea#chatScroll { background: transparent; border: 0; }"));
    m_list->viewport()->setAutoFillBackground(false);
    m_content = new QWidget;
    m_content->setObjectName(QStringLiteral("chatContent"));
    m_content->setAutoFillBackground(false);
    m_listLayout = new QVBoxLayout(m_content);
    m_listLayout->setContentsMargins(kSidePadding, kListTop, kSidePadding, kListBottom);
    m_listLayout->setSpacing(kGap);
    m_list->setWidget(m_content);
    layout->addWidget(m_list, 1);
    connect(m_list->verticalScrollBar(), &QScrollBar::valueChanged, this,
            [this](int value) { m_follow = value >= m_list->verticalScrollBar()->maximum() - 4; });
    // What is said got taller or shorter (it streamed on, or wrapped again as
    // the panel changed width): kept to the latest, it stays at the latest.
    connect(m_list->verticalScrollBar(), &QScrollBar::rangeChanged, this, [this](int, int maximum) {
        if (m_follow) m_list->verticalScrollBar()->setValue(maximum);
    });

    m_composer = new Composer;
    m_composer->setOverlayParent(this);
    connect(m_composer, &Composer::submitRequested, this, &ChatPanel::postRequested);
    connect(m_composer, &Composer::queueRequested, this, &ChatPanel::queueRequested);
    connect(m_composer, &Composer::stopRequested, this, &ChatPanel::stopRequested);
    connect(m_composer, &Composer::suggestionChosen, this, [this](int index) {
        if (index < 0 || index >= m_suggestionList.size()) return;
        const Suggestion suggestion = m_suggestionList.at(index);
        if (suggestion.run) emit runRequested(suggestion.request);
        else m_composer->fill(suggestion.request);
    });
    connect(m_composer, &Composer::escapePressed, this, [this] {
        if (m_composer->isBusy()) emit stopRequested();
        else emit escapeUnused();
    });
    // What a request goes with: the mode, the model and how full the context is.
    m_composer->setAgentControls(true);
    connect(m_composer, &Composer::permissionModeChosen, this, [this](PermissionMode mode) {
        emit permissionModeChosen(mode);
    });
    connect(m_composer, &Composer::modelChosen, this, [this](const QString &model) { emit moreAction(QStringLiteral("model"), model); });
    connect(m_composer, &Composer::agentSettingsRequested, this, [this] { emit moreAction(QStringLiteral("settings"), QString()); });
    layout->addWidget(m_composer);

    m_welcome = buildWelcome();
    rebuild();
    updateHeader();
    updateSuggestions();
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, &ChatPanel::refreshFaces);
}

void ChatPanel::refreshFaces() {
    // Nothing to do while it holds what it was made with.
    const QFont face = tokens::font(reading);
    if (m_face == face) return;
    m_face = face;
    m_composer->refreshFaces();
    // The messages are made again, in the face and size chosen now.
    for (const auto &[itemId, widget] : std::as_const(m_shown))
        if (widget) widget->deleteLater();
    m_shown.clear();
    if (m_strays) {
        m_strays->deleteLater();
        m_strays = nullptr;
    }
    rebuild();
    scheduleFollow();
}

ChatPanel::~ChatPanel() = default;

void ChatPanel::focusComposer() { input()->setFocus(Qt::ShortcutFocusReason); }

QWidget *ChatPanel::buildHeader() {
    auto *header = new QWidget;
    m_header = header;
    header->setObjectName(QStringLiteral("chatHeader"));
    header->setFixedHeight(kHeaderHeight);
    auto *row = new QHBoxLayout(header);
    row->setContentsMargins(space::panelLeft, 0, space::panelHeaderRight, tokens::layout::seam);
    row->setSpacing(space::s8);
    m_title = makeLabel(tokens::title, ink);
    m_title->setObjectName(QStringLiteral("chatTitle"));
    m_title->setTextFormat(Qt::PlainText);
    m_title->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
    m_title->installEventFilter(this);
    row->addWidget(m_title, 1);
    m_titleEdit = new QLineEdit(header);
    m_titleEdit->setObjectName(QStringLiteral("chatTitleEdit"));
    m_titleEdit->setFont(tokens::font(tokens::title));
    m_titleEdit->setFrame(false);
    m_titleEdit->setStyleSheet(QStringLiteral("QLineEdit#chatTitleEdit { background: %1; color: %2; border: 1px solid %3; "
                                              "border-radius: 6px; padding: 1px 6px; selection-background-color: %4; }")
                                   .arg(color(raised).name(), color(ink).name(), color(accent).name(), color(accent).name()));
    m_titleEdit->hide();
    m_titleEdit->installEventFilter(this);
    connect(m_titleEdit, &QLineEdit::editingFinished, this, &ChatPanel::commitRename);
    row->addWidget(m_titleEdit, 1);
    m_more = new ChatIconButton(ChatIconButton::Glyph::More, height::h24);
    m_more->setObjectName(QStringLiteral("chatMore"));
    ui::setTip(m_more, QStringLiteral("More"));
    connect(m_more, &QAbstractButton::clicked, this, [this] {
        QMenu *menu = buildMoreMenu();
        menu->popup(m_more->mapToGlobal(QPoint(m_more->width() - menu->sizeHint().width(), m_more->height() + space::s4)));
    });
    row->addWidget(m_more);
    return header;
}

QWidget *ChatPanel::buildWelcome() {
    // A conversation not yet begun offers where to work and what was last
    // worked on: the folders, the open one ticked, with the ways to another;
    // then the latest conversations here, or things to ask where there are none.
    auto *welcome = new QWidget(m_content);
    welcome->setObjectName(QStringLiteral("chatWelcome"));
    auto *layout = new QVBoxLayout(welcome);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->addStretch(2);
    auto *column = new QWidget;
    column->setFixedWidth(kWelcomeColumn);
    auto *stack = new QVBoxLayout(column);
    stack->setContentsMargins(0, 0, 0, 0);
    stack->setSpacing(space::s12);
    const auto card = [stack](const char *name) {
        auto *frame = new QFrame;
        frame->setObjectName(QString::fromLatin1(name));
        frame->setStyleSheet(QStringLiteral("QFrame#%1 { background: %2; border: 1px solid %3; border-radius: %4px; }")
                                 .arg(QString::fromLatin1(name), color(tokens::card).name(), color(line).name(QColor::HexArgb))
                                 .arg(int(radius::panel)));
        auto *rows = new QVBoxLayout(frame);
        rows->setContentsMargins(space::s4, space::s4, space::s4, space::s4);
        rows->setSpacing(0);
        stack->addWidget(frame);
        return rows;
    };
    m_folderRows = card("welcomeFolders");
    m_examplesLayout = card("welcomeRecent");
    m_examples = m_examplesLayout->parentWidget();
    layout->addWidget(column, 0, Qt::AlignHCenter);
    layout->addStretch(3);
    refillWelcome();
    return welcome;
}

void ChatPanel::refillWelcome() {
    const auto clear = [](QLayout *rows) {
        while (QLayoutItem *item = rows->takeAt(0)) {
            // Out of the card at once, and gone once whatever pressed it is done.
            if (QWidget *widget = item->widget()) {
                widget->hide();
                widget->setParent(nullptr);
                widget->deleteLater();
            }
            delete item;
        }
    };
    // The folders, the open one ticked; under a hairline, the ways to another.
    clear(m_folderRows);
    for (const QString &path : std::as_const(m_folders).first(std::min<qsizetype>(m_folders.size(), kWelcomeFolders))) {
        auto *row = new SidebarRow(QStringLiteral("g-folder"), QFileInfo(path).fileName());
        row->setObjectName(QStringLiteral("welcomeFolder"));
        row->setProperty("folder", path);
        row->setTicked(path == m_folder);
        ui::setTip(row, QDir::toNativeSeparators(path));
        connect(row, &QAbstractButton::clicked, this, [this, path] { emit folderChosen(path); });
        m_folderRows->addWidget(row);
    }
    if (!m_folders.isEmpty()) {
        auto *rule = new QWidget;
        rule->setFixedHeight(tokens::layout::seam + 2 * space::s4);
        rule->setStyleSheet(QStringLiteral("background: qlineargradient(x1:0, y1:0, x2:0, y2:1, stop:0 transparent, stop:0.44 transparent, "
                                           "stop:0.45 %1, stop:0.55 %1, stop:0.56 transparent, stop:1 transparent);")
                                .arg(color(line).name(QColor::HexArgb)));
        m_folderRows->addWidget(rule);
    }
    auto *open = new SidebarRow(QString(), QStringLiteral("Open Folder…"));
    open->setObjectName(QStringLiteral("welcomeOpenFolder"));
    connect(open, &QAbstractButton::clicked, this, &ChatPanel::openFolderRequested);
    m_folderRows->addWidget(open);
    auto *fresh = new SidebarRow(QString(), QStringLiteral("New Project…"));
    fresh->setObjectName(QStringLiteral("welcomeNewProject"));
    connect(fresh, &QAbstractButton::clicked, this, &ChatPanel::newProjectRequested);
    m_folderRows->addWidget(fresh);

    // The latest conversations here; with none yet, things to ask.
    clear(m_examplesLayout);
    m_exampleButtons.clear();
    QList<ConversationModel::Summary> recent;
    if (m_conversations)
        for (const ConversationModel::Summary &summary : m_conversations->list())
            if (!summary.archived && summary.id != m_conversations->currentId()) recent << summary;
    std::stable_sort(recent.begin(), recent.end(),
                     [](const ConversationModel::Summary &a, const ConversationModel::Summary &b) { return a.updated > b.updated; });
    for (const ConversationModel::Summary &summary : std::as_const(recent).first(std::min<qsizetype>(recent.size(), kWelcomeRecent))) {
        const QString title = summary.title.isEmpty() ? QStringLiteral("Conversation") : summary.title;
        auto *row = new SidebarRow(QString(), title);
        row->setObjectName(QStringLiteral("welcomeConversation"));
        ui::setTip(row, title);
        connect(row, &QAbstractButton::clicked, this, [this, id = summary.id] { emit conversationRequested(id); });
        m_examplesLayout->addWidget(row);
    }
    if (!recent.isEmpty()) return;
    QList<QPair<QString, QString>> offered;
    for (const QJsonValue &value : std::as_const(m_steps)) {
        const QJsonObject step = value.toObject();
        const QString label = value.isString() ? value.toString() : step.value(QStringLiteral("label")).toString();
        const QString request = value.isString() ? value.toString() : step.value(QStringLiteral("request")).toString();
        if (!label.isEmpty()) offered.append({label, request.isEmpty() ? label : request});
    }
    if (offered.isEmpty())
        for (const QString &request : firstRequests()) offered.append({request, request});
    for (const auto &[label, request] : std::as_const(offered).first(std::min<qsizetype>(offered.size(), 4))) {
        auto *button = new SuggestionButton(label);
        button->setObjectName(QStringLiteral("welcomeExample"));
        button->setRequest(request);
        connect(button, &QAbstractButton::clicked, this, [this, button] { m_composer->fill(button->request()); });
        m_examplesLayout->addWidget(button);
        m_exampleButtons << button;
    }
}

void ChatPanel::setFolders(const QStringList &folders, const QString &current) {
    if (folders == m_folders && current == m_folder) return;
    m_folders = folders;
    m_folder = current;
    refillWelcome();
}

void ChatPanel::setPermissionMode(PermissionMode mode) {
    m_composer->setPermissionMode(mode);
}

PermissionMode ChatPanel::permissionMode() const { return m_composer->permissionMode(); }

void ChatPanel::setContextUse(const QString &conversation, qint64 used, qint64 limit, bool tokens) {
    m_context = ContextUse{conversation, used, limit, tokens};
    updateContextMeter();
}

void ChatPanel::updateContextMeter() {
    ContextMeter *meter = m_composer->contextMeter();
    const QString open = m_conversations ? m_conversations->currentId() : QString();
    // Said of a conversation just started, it is of the one that is open as
    // soon as the bridge names it.
    if (m_context.conversation.isEmpty() && !open.isEmpty() && (m_live.active || m_live.finished) && m_live.conversation == open)
        m_context.conversation = open;
    const bool here = m_context.limit > 0 && (m_context.conversation == open || (m_context.conversation.isEmpty() && m_live.active));
    if (here) meter->setUse(m_context.used, m_context.limit, m_context.tokens);
    meter->setVisible(here);
}

QList<QAbstractButton *> ChatPanel::examples() const {
    QList<QAbstractButton *> out;
    for (SuggestionButton *button : m_exampleButtons) out << button;
    return out;
}

void ChatPanel::updateHeader() {
    QString title;
    if (m_conversations && !m_conversations->currentId().isEmpty()) title = m_conversations->current().summary.title;
    if (title.isEmpty() && (m_live.active || m_live.finished)) title = chat::titleLine(m_live.text);
    if (title.isEmpty()) title = QStringLiteral("New conversation");
    m_title->setText(QFontMetricsF(m_title->font()).elidedText(title, Qt::ElideRight, std::max(40, m_title->width())));
    m_title->setToolTip(title);
    const bool changed = title != titleText();
    m_title->setProperty("fullTitle", title);
    if (changed) emit titleChanged();
}

QString ChatPanel::titleText() const { return m_title->property("fullTitle").toString(); }

void ChatPanel::setHeaderShown(bool shown) {
    if (shown == m_headerShown) return;
    m_headerShown = shown;
    m_header->setVisible(shown);
    update();
}

void ChatPanel::startRename() {
    if (!m_conversations || m_conversations->currentId().isEmpty()) return;
    m_titleEdit->setText(m_conversations->current().summary.title);
    m_title->hide();
    m_titleEdit->show();
    m_titleEdit->selectAll();
    m_titleEdit->setFocus(Qt::OtherFocusReason);
    emit titleChanged();
}

void ChatPanel::commitRename() {
    if (m_titleEdit->isHidden()) return;
    const QString title = m_titleEdit->text().simplified();
    m_titleEdit->hide();
    m_title->show();
    const bool renamed = m_conversations && !m_conversations->currentId().isEmpty() && !title.isEmpty() &&
                         title != m_conversations->current().summary.title;
    if (renamed) {
        // Shown at once; the bridge's next list has the last word.
        m_title->setText(QFontMetricsF(m_title->font()).elidedText(title, Qt::ElideRight, std::max(40, m_title->width())));
        m_title->setProperty("fullTitle", title);
    }
    emit titleChanged();
    if (renamed) emit conversationAction(QStringLiteral("rename"), title);
}

// ---- inputs ------------------------------------------------------------------------

void ChatPanel::setConversations(ConversationModel *model) {
    if (m_conversations == model) return;
    if (m_conversations) disconnect(m_conversations, nullptr, this, nullptr);
    m_conversations = model;
    if (model) {
        connect(model, &ConversationModel::currentChanged, this, [this] {
            updateHeader();
            rebuild();
            updateContextMeter();
            scheduleFollow();
        });
        connect(model, &ConversationModel::listChanged, this, [this] {
            updateHeader();
            refillWelcome();
        });
    }
    updateHeader();
    rebuild();
}

void ChatPanel::setProposals(ProposalModel *model) {
    if (m_model == model) return;
    if (m_model) disconnect(m_model, nullptr, this, nullptr);
    m_model = model;
    if (model) connect(model, &ProposalModel::changed, this, &ChatPanel::scheduleRebuild);
    scheduleRebuild();
}

void ChatPanel::setSnapshot(const QJsonObject &snapshot) {
    m_timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    m_proposalTimelines = snapshot.value(QStringLiteral("proposalTimelines")).toObject();
    const QJsonValue fps = m_timeline.value(QStringLiteral("fps"));
    if (fps.isDouble() && fps.toDouble() > 0) m_fps = fps.toDouble();
    scheduleRebuild();
}

void ChatPanel::setNamedTimeline(const QJsonObject &timeline) {
    if (timeline == m_named) return;
    m_named = timeline;
    scheduleRebuild();
}

void ChatPanel::setFps(double fps) {
    if (fps <= 0 || fps == m_fps) return;
    m_fps = fps;
    scheduleRebuild();
}

void ChatPanel::setConnected(bool connected) {
    if (connected == m_connected) return;
    m_connected = connected;
    scheduleRebuild();
}

void ChatPanel::setModels(const QStringList &models, const QString &current, bool choosable) {
    m_models = models;
    m_currentModel = current;
    m_modelsChoosable = choosable;
    m_composer->setModels(models, current, choosable);
}

void ChatPanel::setNextSteps(const QJsonArray &steps) {
    m_steps = steps;
    refillWelcome();
    updateSuggestions();
}

void ChatPanel::setCheckRows(const QJsonArray &rows) {
    m_checkRows = rows;
    updateSuggestions();
}

void ChatPanel::setPreviewProposal(const QString &proposalId) {
    if (proposalId == m_previewProposal) return;
    m_previewProposal = proposalId;
    for (ProposalCard *card : cards()) {
        const ProposalModel::Proposal *proposal = m_model ? m_model->proposal(card->proposalId()) : nullptr;
        card->setPreview(!proposalId.isEmpty() && proposal && proposal->hasVersion(proposalId));
    }
}

void ChatPanel::markSettledHere(const QString &proposalId, bool accepted) {
    m_settledHere.insert(proposalId, accepted);
    m_settledAt.insert(proposalId, QDateTime::currentDateTime());
    scheduleRebuild();
}

void ChatPanel::setAcceptAction(QAction *action) {
    m_acceptAction = action;
    if (!action) return;
    // The action changes whenever it is enabled or not; the cards only care about its key.
    const auto follow = [this, action, said = std::make_shared<QString>()] {
        const QString key = ui::keyText(action->shortcut());
        if (key == *said) return;
        *said = key;
        for (ProposalCard *card : cards()) card->setAcceptKey(key);
    };
    connect(action, &QAction::changed, this, follow);
    follow();
}

bool ChatPanel::onStage(const ProposalModel::Proposal &proposal) const {
    return !m_stageProposal.isEmpty() && (proposal.id == m_stageProposal || proposal.hasVersion(m_stageProposal));
}

void ChatPanel::refreshPictures() {
    if (!m_pictureSource || !m_model) return;
    for (ProposalCard *card : cards())
        if (!card->isSettled()) card->setPicture(m_pictureSource(card->shownId()));
}

void ChatPanel::setStageProposal(const QString &proposalId) {
    m_stageProposal = proposalId;
    for (ProposalCard *card : cards()) {
        const ProposalModel::Proposal *proposal = m_model ? m_model->proposal(card->proposalId()) : nullptr;
        card->picture()->setOnStage(proposal && onStage(*proposal));
    }
}

// ---- the turn in progress ---------------------------------------------------------

YouMessage *ChatPanel::makeYou(const QString &text, double at) {
    auto *you = new YouMessage(text, at, m_fps, m_content);
    connect(you, &YouMessage::seekRequested, this, &ChatPanel::seekRequested);
    moves::fadeIn(you);
    return you;
}

KerfMessage *ChatPanel::makeKerf() {
    auto *kerf = new KerfMessage(m_fps, m_content);
    connect(kerf, &KerfMessage::seekRequested, this, &ChatPanel::seekRequested);
    connect(kerf, &KerfMessage::askAgain, this, &ChatPanel::askAgain);
    connect(kerf->work()->toggle(), &QAbstractButton::toggled, this, [this] { syncContent(); });
    return kerf;
}

void ChatPanel::beginTurn(const QString &text, double at) {
    dropLive();
    m_live.active = true;
    m_live.conversation = m_conversations ? m_conversations->currentId() : QString();
    m_live.base = m_conversations && !m_live.conversation.isEmpty() ? m_conversations->current().items.size() : 0;
    m_live.text = text.trimmed();
    m_live.at = at;
    m_live.you = makeYou(m_live.text, at);
    KerfMessage *kerf = ensureLiveKerf();
    kerf->setDrafting(true);
    m_follow = true;
    updateHeader();
    rebuild();
    scheduleFollow();
}

KerfMessage *ChatPanel::ensureLiveKerf() {
    if (!m_live.kerf) {
        m_live.kerf = makeKerf();
        m_live.kerf->work()->start();
        if (!m_live.active) m_live.active = true;
        rebuild();
    }
    return m_live.kerf;
}

void ChatPanel::dropLive() {
    if (m_live.you) m_live.you->deleteLater();
    if (m_live.kerf) {
        // The building card goes with it: the cut it was making is made.
        if (m_buildingCard && m_live.kerf->isAncestorOf(m_buildingCard)) {
            m_justBuilt = m_building;
            m_builtAt = CardPicture::clock();
            m_building.clear();
            m_buildingCard = nullptr;
        }
        m_live.kerf->deleteLater();
    }
    m_live = LiveTurn{};
}

void ChatPanel::appendAnswer(const QString &delta) {
    if (delta.isEmpty()) return;
    KerfMessage *kerf = ensureLiveKerf();
    kerf->appendText(delta);
    scheduleFollow();
}

void ChatPanel::setAnswer(const QString &text) {
    if (text.isEmpty()) return;
    KerfMessage *kerf = ensureLiveKerf();
    kerf->setText(text);
    scheduleFollow();
}

void ChatPanel::addStep(const QString &step) {
    if (!m_live.active) return;
    ensureLiveKerf()->work()->addStep(step);
    scheduleFollow();
}

void ChatPanel::setStep(const QString &id, const QString &step, bool failed) {
    if (!m_live.active) return;
    ensureLiveKerf()->work()->setStep(id, step, failed);
    scheduleFollow();
}

void ChatPanel::addAsk(const MakeCard::Ask &ask) {
    if (!m_live.active && !m_live.finished) return;
    ensureLiveKerf()->addAsk(makeAskCard(ask));
    scheduleFollow();
}

MakeCard *ChatPanel::makeAskCard(const MakeCard::Ask &ask) {
    auto *card = new MakeCard(ask);
    connect(card, &MakeCard::makeRequested, this, [this](const QString &media, int rolls) {
        m_makings.insert(media, Making{MakeCard::State::Making, QString(), 0});
        emit makeRequested(media, rolls);
    });
    connect(card, &MakeCard::stopRequested, this, &ChatPanel::stopMakingRequested);
    connect(card, &MakeCard::lookRequested, this, &ChatPanel::lookAtTakeRequested);
    connect(card, &MakeCard::declined, this, [this](const QString &key) { m_declined.insert(key); });
    syncAskCard(card);
    return card;
}

void ChatPanel::syncAskCard(MakeCard *card) const {
    const MakeCard::Ask &ask = card->ask();
    const MediaFacts facts = m_mediaFacts ? m_mediaFacts(ask.media, ask.take) : MediaFacts{};
    card->setMediaName(facts.name);
    card->setCanMake(m_connected);
    if (facts.made) {
        card->setState(MakeCard::State::Made, QString(), ask.take + std::max(1, ask.rolls) - 1);
    } else if (const auto making = m_makings.constFind(ask.media); making != m_makings.cend()) {
        card->setState(making->state, making->detail, making->take);
    } else if (m_declined.contains(ask.key())) {
        card->setState(MakeCard::State::Declined);
    }
}

void ChatPanel::setMakingState(const QString &media, MakeCard::State state, const QString &detail, int take) {
    m_makings.insert(media, Making{state, detail, take});
    for (MakeCard *card : makeCards())
        if (card->ask().media == media) card->setState(state, detail, take);
}

QList<MakeCard *> ChatPanel::makeCards() const {
    QList<MakeCard *> out;
    for (KerfMessage *kerf : answers()) out << kerf->asks();
    return out;
}

void ChatPanel::setTurnStatus(const QString &status) {
    if (!m_live.active || !m_live.kerf) return;
    m_live.kerf->work()->setStatus(status);
}

void ChatPanel::setTurnBusy(bool busy) {
    m_composer->setBusy(busy);
    if (!busy && m_live.active) finishTurn();
}

void ChatPanel::finishTurn() {
    if (!m_live.active) return;
    m_live.active = false;
    m_live.finished = true;
    if (KerfMessage *kerf = m_live.kerf) {
        kerf->work()->finish(0);
        kerf->setDrafting(false);
        // The cut it was making is what it proposed.
        if (!m_building.isEmpty() && kerf->proposal().isEmpty()) kerf->setProposal(m_building);
    }
    m_composer->setBusy(false);
    rebuild();
}

void ChatPanel::failTurn(const QString &message) {
    KerfMessage *kerf = ensureLiveKerf();
    if (!message.isEmpty()) kerf->setText(message);
    kerf->setOutcome(KerfMessage::Outcome::Failed, m_live.text);
    finishTurn();
}

void ChatPanel::stopTurn() {
    if (!m_live.active && !m_live.finished) return;
    KerfMessage *kerf = ensureLiveKerf();
    kerf->setOutcome(KerfMessage::Outcome::Stopped, m_live.text);
    finishTurn();
}

// ---- the list -----------------------------------------------------------------------

void ChatPanel::scheduleRebuild() {
    if (m_rebuildQueued) return;
    m_rebuildQueued = true;
    QTimer::singleShot(0, this, [this] {
        if (m_rebuildQueued) rebuild();
    });
}

void ChatPanel::insertMessage(QWidget *message) {
    message->setParent(m_content);
    m_listLayout->addWidget(message);
    message->show();
}

void ChatPanel::rebuild() {
    m_rebuildQueued = false;
    static const ConversationModel::Conversation none;
    const ConversationModel::Conversation &conversation = m_conversations ? m_conversations->current() : none;
    const QString id = conversation.summary.id;

    // Another conversation: what the last one showed goes.
    if (id != m_shownConversation) {
        for (const auto &[itemId, widget] : std::as_const(m_shown))
            if (widget) widget->deleteLater();
        m_shown.clear();
        m_shownConversation = id;
    }

    // The live turn shows in the conversation it was asked in; asked in a new
    // one, in the one it started.
    bool live = m_live.active || m_live.finished;
    if (live && m_live.conversation.isEmpty() && !id.isEmpty() && m_conversations && !m_conversations->expectsNew()) {
        for (const ConversationModel::Item &item : conversation.items)
            if (item.you && item.text.trimmed() == m_live.text) m_live.conversation = id;
    }
    if (live && m_live.conversation != id) live = false;
    // The bridge kept the turn: its request and answer are in the conversation.
    qsizetype liveYou = -1;
    if (live) {
        // Its request, kept after what the conversation held when it was asked
        // (an earlier one asked the same way is not it).
        for (qsizetype index = conversation.items.size() - 1; index >= m_live.base; --index)
            if (conversation.items.at(index).you && conversation.items.at(index).text.trimmed() == m_live.text) {
                liveYou = index;
                break;
            }
        if (liveYou >= 0 && liveYou + 1 < conversation.items.size()) {
            // And its answer after it: kept whole once the turn is over.
            if (m_live.finished) {
                dropLive();
                live = false;
            }
            liveYou = -1;
        }
    }

    // The kept messages: what is already shown stays, the rest is made.
    qsizetype keep = 0;
    while (keep < m_shown.size() && keep < conversation.items.size() && m_shown.at(keep).first == conversation.items.at(keep).id &&
           m_shown.at(keep).second)
        ++keep;
    for (qsizetype index = keep; index < m_shown.size(); ++index)
        if (m_shown.at(index).second) m_shown.at(index).second->deleteLater();
    m_shown.resize(keep);
    QString request;
    for (qsizetype index = 0; index < conversation.items.size(); ++index) {
        const ConversationModel::Item &item = conversation.items.at(index);
        if (item.you) request = item.text;
        if (index < keep) continue;
        QWidget *widget = nullptr;
        if (item.you && index == liveYou && m_live.you) {
            // The request shown at once is the one kept.
            widget = m_live.you;
            m_live.you = nullptr;
        } else if (item.you) {
            widget = makeYou(item.text, item.timecode);
        } else {
            KerfMessage *kerf = makeKerf();
            kerf->work()->setSteps(item.steps);
            kerf->work()->finish(item.seconds);
            kerf->setText(item.text);
            kerf->setOutcome(item.error     ? KerfMessage::Outcome::Failed
                             : item.stopped ? KerfMessage::Outcome::Stopped
                                            : KerfMessage::Outcome::Answer,
                             request);
            kerf->setProposal(item.proposal);
            for (const MakeCard::Ask &ask : item.asks) kerf->addAsk(makeAskCard(ask));
            widget = kerf;
        }
        m_shown.append({item.id, widget});
    }

    // Laid out again, in order.
    while (QLayoutItem *item = m_listLayout->takeAt(0)) delete item;
    const bool empty = m_shown.isEmpty() && !live;
    m_welcome->setVisible(empty);
    if (empty) m_listLayout->addWidget(m_welcome, 1);
    for (const auto &[itemId, widget] : std::as_const(m_shown))
        if (widget) insertMessage(widget);
    if (live) {
        if (m_live.you) insertMessage(m_live.you);
        if (m_live.kerf) insertMessage(m_live.kerf);
    } else {
        if (m_live.you) m_live.you->hide();
        if (m_live.kerf) m_live.kerf->hide();
    }
    refreshCards();
    for (MakeCard *card : makeCards()) syncAskCard(card);
    if (m_strays && !m_strays->cards().isEmpty()) insertMessage(m_strays);
    else if (m_strays) m_strays->hide();
    if (m_footage && !m_footage->isHidden()) m_listLayout->addWidget(m_footage);
    if (!empty) m_listLayout->addStretch(1);
    syncContent();
}

QStringList ChatPanel::strayProposals() const {
    if (!m_model) return {};
    QStringList staged;
    for (const auto &[itemId, widget] : m_shown)
        if (auto *kerf = qobject_cast<KerfMessage *>(widget.data()); kerf && !kerf->proposal().isEmpty()) staged << kerf->proposal();
    if (m_live.kerf && !m_live.kerf->proposal().isEmpty()) staged << m_live.kerf->proposal();
    QStringList out;
    for (const ProposalModel::Proposal &proposal : m_model->openProposals()) {
        if (!m_building.isEmpty() && (proposal.id == m_building || proposal.hasVersion(m_building))) continue;
        const bool here = std::any_of(staged.cbegin(), staged.cend(), [&](const QString &id) { return proposal.hasVersion(id) || proposal.id == id; });
        // Staged by an agent over MCP, or in a conversation not open now: it
        // waits on the person all the same.
        if (!here) out << proposal.id;
    }
    return out;
}

void ChatPanel::refreshCards() {
    // What a card is made from, so a message whose cards would come out the
    // same keeps them.
    const auto keyFor = [this](const QStringList &ids) {
        QByteArray key;
        QDataStream out(&key, QIODevice::WriteOnly);
        out << m_connected << m_previewProposal << m_stageProposal << m_fps;
        for (const QString &id : ids) {
            const ProposalModel::Proposal *proposal = m_model ? m_model->proposal(id) : nullptr;
            out << id;
            if (!proposal) continue;
            out << QJsonDocument(proposal->json).toJson(QJsonDocument::Compact) << proposal->status << proposal->stale
                << m_settledHere.contains(proposal->id) << m_cardState.value(proposal->id).shown
                << QJsonDocument(m_proposalTimelines.value(proposal->id).toObject()).toJson(QJsonDocument::Compact);
        }
        out << QJsonDocument(m_timeline).toJson(QJsonDocument::Compact) << QJsonDocument(m_named).toJson(QJsonDocument::Compact);
        return key;
    };
    const auto fill = [this, &keyFor](KerfMessage *kerf, const QStringList &ids) {
        const QByteArray key = keyFor(ids);
        if (kerf->property("cardKey").toByteArray() == key && kerf->cards().size() <= ids.size()) return;
        kerf->setProperty("cardKey", key);
        kerf->clearCards();
        for (const QString &id : ids) {
            const ProposalModel::Proposal *proposal = m_model ? m_model->proposal(id) : nullptr;
            if (!proposal) continue;
            if (ProposalCard *card = buildCard(*proposal)) kerf->addCard(card);
        }
    };
    for (const auto &[itemId, widget] : std::as_const(m_shown)) {
        auto *kerf = qobject_cast<KerfMessage *>(widget.data());
        if (!kerf || kerf->proposal().isEmpty()) continue;
        fill(kerf, {kerf->proposal()});
    }
    // The finished turn's, until the bridge keeps it.
    if (m_live.kerf && m_live.finished && !m_live.kerf->proposal().isEmpty()) fill(m_live.kerf, {m_live.kerf->proposal()});
    const QStringList strays = strayProposals();
    if (strays.isEmpty()) {
        if (m_strays) m_strays->clearCards();
        return;
    }
    if (!m_strays) {
        m_strays = makeKerf();
        m_strays->setObjectName(QStringLiteral("waitingProposals"));
    }
    // Who proposed it, when one agent did.
    QString text = strays.size() == 1 ? QStringLiteral("Proposed outside this conversation.")
                                      : QStringLiteral("%1 proposals made outside this conversation.").arg(strays.size());
    if (const ProposalModel::Proposal *only = strays.size() == 1 ? m_model->proposal(strays.constFirst()) : nullptr;
        only && only->author.isAgent() && !only->author.name.isEmpty())
        text = QStringLiteral("%1 proposed this over MCP.").arg(only->author.name);
    if (m_strays->text() != text) m_strays->setText(text);
    fill(m_strays, strays);
}

QList<QWidget *> ChatPanel::messages() const {
    QList<QWidget *> out;
    for (int index = 0; index < m_listLayout->count(); ++index) {
        QWidget *widget = m_listLayout->itemAt(index)->widget();
        if (qobject_cast<YouMessage *>(widget) || qobject_cast<KerfMessage *>(widget)) out << widget;
    }
    return out;
}

QList<YouMessage *> ChatPanel::requests() const {
    QList<YouMessage *> out;
    for (QWidget *widget : messages())
        if (auto *you = qobject_cast<YouMessage *>(widget)) out << you;
    return out;
}

QList<KerfMessage *> ChatPanel::answers() const {
    QList<KerfMessage *> out;
    for (QWidget *widget : messages())
        if (auto *kerf = qobject_cast<KerfMessage *>(widget)) out << kerf;
    return out;
}

QString ChatPanel::transcript() const {
    QStringList lines;
    for (QWidget *widget : messages()) {
        if (auto *you = qobject_cast<YouMessage *>(widget)) lines << QStringLiteral("You: %1").arg(you->text());
        else if (auto *kerf = qobject_cast<KerfMessage *>(widget); kerf && !kerf->text().isEmpty())
            lines << QStringLiteral("Kerf: %1").arg(kerf->text());
    }
    return lines.join(QLatin1Char('\n'));
}

ProposalCard *ChatPanel::card(const QString &proposalId) const {
    const ProposalModel::Proposal *proposal = m_model ? m_model->proposal(proposalId) : nullptr;
    for (ProposalCard *card : cards())
        if (card->proposalId() == proposalId || (proposal && card->proposalId() == proposal->id)) return card;
    return nullptr;
}

QList<ProposalCard *> ChatPanel::cards() const {
    QList<ProposalCard *> out;
    for (ProposalCard *card : m_content->findChildren<ProposalCard *>())
        if (!card->isHidden() || card == m_buildingCard) out << card;
    return out;
}

QString ChatPanel::shownVersion(const QString &proposalId) const {
    const QString shown = m_cardState.value(proposalId).shown;
    const ProposalModel::Proposal *proposal = m_model && !shown.isEmpty() ? m_model->proposal(proposalId) : nullptr;
    return proposal && proposal->hasVersion(shown) ? shown : proposalId;
}

QString ChatPanel::trackName(const QString &trackId) const {
    for (const QJsonObject *timeline : {&m_named, &m_timeline})
        for (const QJsonValue &value : timeline->value(QStringLiteral("tracks")).toArray()) {
            const QJsonObject track = value.toObject();
            if (track.value(QStringLiteral("id")).toString() != trackId) continue;
            const QString name = track.value(QStringLiteral("name")).toString();
            if (!name.isEmpty()) return name;
        }
    return trackId;
}

QString ChatPanel::clipName(const QString &clipId) const {
    for (const QJsonObject *timeline : {&m_named, &m_timeline}) {
        const QString name = findClip(*timeline, clipId).value(QStringLiteral("name")).toString();
        if (!name.isEmpty()) return name;
    }
    return clipId;
}

QList<ProposalModel::ClipChange> ChatPanel::versionChanges(const ProposalModel::Proposal &proposal, const QString &versionId) const {
    if (versionId.isEmpty() || versionId == proposal.id) {
        if (!proposal.changes.isEmpty() || !m_proposalTimelines.contains(proposal.id)) return proposal.changes;
    }
    const QString id = versionId.isEmpty() ? proposal.id : versionId;
    if (!m_proposalTimelines.contains(id)) return {};
    return ProposalModel::diffTimelines(m_timeline, m_proposalTimelines.value(id).toObject());
}

ChatPanel::ColourChange ChatPanel::colourChange(const ProposalModel::Proposal &proposal, const QString &versionId) const {
    ColourChange out;
    const QList<ProposalModel::ClipChange> changes = versionChanges(proposal, versionId);
    if (changes.size() != 1) return out;
    const ProposalModel::ClipChange &change = changes.constFirst();
    if (!change.effects || change.transition || change.added || change.removed) return out;
    const QJsonObject was = findClip(m_timeline, change.clip);
    const QJsonObject now = findClip(m_proposalTimelines.value(versionId).toObject(), change.clip);
    if (now.isEmpty()) return out;
    const QJsonArray before = was.value(QStringLiteral("effects")).toArray();
    const QJsonArray after = now.value(QStringLiteral("effects")).toArray();
    for (qsizetype index = 0; index < after.size(); ++index) {
        const QJsonObject effect = after.at(index).toObject();
        const QString fx = effect.value(QStringLiteral("fx")).toString();
        QJsonObject old;
        bool had = false;
        for (qsizetype other = 0; other < before.size(); ++other) {
            const QJsonObject candidate = before.at(other).toObject();
            if (candidate.value(QStringLiteral("fx")).toString() == fx) {
                old = candidate;
                had = true;
                break;
            }
        }
        const QJsonObject oldParams = staticParams(old.value(QStringLiteral("params")).toObject());
        const QJsonObject newParams = staticParams(effect.value(QStringLiteral("params")).toObject());
        if (had && oldParams == newParams) continue;
        if (!isColourEffect(fx)) return out;
        out.clip = change.clip;
        out.fx = fx;
        out.title = QStringLiteral("%1 · %2").arg(change.name.isEmpty() ? clipName(change.clip) : change.name, effectLabel(fx));
        QStringList parts;
        for (auto it = newParams.begin(); it != newParams.end(); ++it) {
            if (had && oldParams.value(it.key()) == it.value()) continue;
            const QString value = paramValue(it.value());
            if (!value.isEmpty()) parts << QStringLiteral("%1 %2").arg(effectLabel(it.key()), value);
        }
        out.params = parts.join(QStringLiteral(", "));
        return out;
    }
    return out;
}

ProposalCard *ChatPanel::buildCard(const ProposalModel::Proposal &proposal) {
    auto *card = new ProposalCard(proposal.id);
    card->setCompact(true);
    moves::fadeIn(card);
    if (m_acceptAction) card->setAcceptKey(ui::keyText(m_acceptAction->shortcut()));
    const QList<ProposalModel::Proposal> open = m_model->openProposals();
    const bool pinned = proposal.isOpen() && !open.isEmpty() && open.constFirst().id == proposal.id;
    CardState &state = m_cardState[proposal.id];
    if (!state.openSet) {
        state.open = false;
        state.openSet = true;
    }
    QString shown = state.shown;
    if (!shown.isEmpty() && !proposal.hasVersion(shown)) shown.clear();
    const QString versionId = shown.isEmpty() ? proposal.id : shown;
    card->setPinned(pinned);
    card->setTitle(proposal.summary);
    // A cut Kerf made shows its picture; pressing it opens the stage.
    if (m_pictureSource && proposal.isOpen()) {
        card->setPicture(m_pictureSource(versionId));
        card->picture()->setOnStage(onStage(proposal));
        // Just made: it shows it is done, from where it has got to.
        if (proposal.hasVersion(m_justBuilt) || proposal.id == m_justBuilt) {
            const qint64 ago = CardPicture::clock() - m_builtAt;
            if (ago < tokens::motion::cardDone) card->picture()->setFinished(ago);
            else m_justBuilt.clear();
        }
        connect(card, &ProposalCard::pictureRequested, this, [this, id = proposal.id] { emit pictureRequested(id); });
    }

    // The changes: from the timelines when there are some, else the script's words.
    QList<ProposalCard::Change> rows;
    for (const ProposalModel::ClipChange &change : versionChanges(proposal, versionId)) {
        ProposalCard::Change row;
        const QString name = change.transition ? QStringLiteral("Transition") : (change.name.isEmpty() ? clipName(change.clip) : change.name);
        row.name = change.track.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, trackName(change.track));
        if (change.removed) {
            row.target = QStringLiteral("removed");
        } else if (change.added) {
            row.target = QStringLiteral("→ %1").arg(chat::timecode(change.newStart, m_fps));
            row.shift = QStringLiteral("new");
        } else if (change.moved) {
            row.target = QStringLiteral("→ %1").arg(chat::timecode(change.newStart, m_fps));
            row.shift = chat::signedFrames(std::llround(change.shift() * m_fps));
        } else if (change.trimmed) {
            row.target = QStringLiteral("→ %1").arg(chat::timecode(change.newEnd, m_fps));
            row.shift = chat::signedFrames(std::llround(((change.newEnd - change.newStart) - (change.oldEnd - change.oldStart)) * m_fps));
        } else {
            QStringList what;
            if (change.effects) what << QStringLiteral("effects");
            if (change.properties) what << QStringLiteral("properties");
            if (change.toggled) what << QStringLiteral("enabled");
            if (change.retimed) what << QStringLiteral("speed");
            if (change.relinked) what << QStringLiteral("link");
            if (change.retracked) what << QStringLiteral("track");
            row.target = what.join(QStringLiteral(", "));
        }
        rows << row;
    }
    if (rows.isEmpty() && m_describer) {
        QString script = proposal.script;
        for (const ProposalModel::Version &version : proposal.versions)
            if (version.proposalId == versionId) script = version.script;
        for (const QString &line : script.split(QLatin1Char('\n'))) {
            const QString text = line.trimmed();
            if (text.isEmpty() || text.startsWith(QLatin1Char('#')) || text.startsWith(QLatin1String("use "))) continue;
            const ProposalChange change = m_describer(text);
            rows << ProposalCard::Change{change.description, change.detail, QString()};
        }
    }
    card->setChanges(rows);

    // Versions, each with where its first move lands.
    QList<ProposalCard::VersionItem> versions;
    for (const ProposalModel::Version &version : proposal.versions) {
        ProposalCard::VersionItem item{version.version, version.proposalId, QStringLiteral("v%1").arg(version.version)};
        for (const ProposalModel::ClipChange &change : versionChanges(proposal, version.proposalId))
            if (change.moved) {
                item.label = QStringLiteral("v%1 %2 · %3 fr")
                                 .arg(version.version)
                                 .arg(chat::timecode(change.newStart, m_fps))
                                 .arg(chat::signedFrames(std::llround(change.shift() * m_fps)).chopped(1));
                break;
            }
        versions << item;
    }
    card->setVersions(versions, versionId);

    // A colour change on one clip has a line of its own.
    const ColourChange colour = colourChange(proposal, versionId);
    if (!colour.clip.isEmpty()) card->setColour(ProposalCard::Colour{colour.clip, colour.fx, colour.title, colour.params});

    // The diff: the document's changed lines, else the script.
    QString diff = proposal.documentDiff;
    QString script = proposal.script;
    for (const ProposalModel::Version &version : proposal.versions)
        if (version.proposalId == versionId) {
            diff = version.documentDiff;
            script = version.script;
        }
    QStringList lines;
    for (const QString &line : diff.split(QLatin1Char('\n'))) {
        if (line.startsWith(QLatin1String("+++")) || line.startsWith(QLatin1String("---"))) continue;
        if (line.startsWith(QLatin1Char('+')) || line.startsWith(QLatin1Char('-'))) lines << line;
    }
    if (lines.isEmpty())
        for (const QString &line : script.split(QLatin1Char('\n')))
            if (!line.trimmed().isEmpty()) lines << line;
    card->setDiff(lines);

    card->setOpen(state.open);
    card->setScriptShown(state.script);
    card->setPreview(!m_previewProposal.isEmpty() && proposal.hasVersion(m_previewProposal));
    {
        double end = 0.0;
        for (const ProposalModel::ClipChange &change : proposal.changes) end = std::max(end, change.removed ? change.oldEnd : change.newEnd);
        card->setWatchLength(reel::total(reel::plan(proposal.changes, end, end)));
    }
    card->setPrimary(pinned);
    card->setStale(proposal.isOpen() && proposal.stale);
    if (!m_connected) card->setActionsEnabled(false);
    if (!proposal.isOpen()) {
        const bool accepted = proposal.status == QLatin1String("accepted");
        QString settledKey;
        for (auto it = m_settledHere.cbegin(); it != m_settledHere.cend(); ++it)
            if (proposal.hasVersion(it.key()) || it.key() == proposal.id) settledKey = it.key();
        QString text = accepted ? (proposal.appliedRevision >= 0 ? QStringLiteral("Accepted in rev %1").arg(proposal.appliedRevision)
                                                                 : QStringLiteral("Accepted"))
                                : QStringLiteral("Rejected");
        if (!settledKey.isEmpty())
            text += QStringLiteral(" · %1").arg(chat::age(m_settledAt.value(settledKey), QDateTime::currentDateTime()));
        card->setSettled(text, accepted);
    }

    const QString id = proposal.id;
    connect(card, &ProposalCard::openToggled, this, [this, id](bool openNow) {
        m_cardState[id].open = openNow;
        syncContent();
    });
    connect(card, &ProposalCard::scriptToggled, this, [this, id](bool shownNow) { m_cardState[id].script = shownNow; });
    connect(card, &ProposalCard::previewToggled, this, [this, card](bool on) { emit previewRequested(card->shownId(), on); });
    connect(card, &ProposalCard::versionChosen, this, [this, id](const QString &version) {
        m_cardState[id].shown = version == id ? QString() : version;
        // Choosing an older version previews it.
        emit previewRequested(version, true);
        emit shownVersionChanged(id, version);
        scheduleRebuild();
    });
    connect(card, &ProposalCard::acceptRequested, this, &ChatPanel::acceptRequested);
    connect(card, &ProposalCard::forwardRequested, this, &ChatPanel::forwardRequested);
    connect(card, &ProposalCard::watchRequested, this, [this, card] { emit changedPartsRequested(card->shownId()); });
    connect(card, &ProposalCard::rejectRequested, this, &ChatPanel::rejectRequested);
    return card;
}

void ChatPanel::setBuildingProposal(const QString &proposalId) {
    const ProposalModel::Proposal *proposal = m_model && !proposalId.isEmpty() ? m_model->proposal(proposalId) : nullptr;
    if (proposalId == m_building && (m_buildingCard || proposalId.isEmpty())) {
        // The same cut, further on: its picture becomes the newest clip.
        if (m_buildingCard) {
            if (m_pictureSource) m_buildingCard->setPicture(m_pictureSource(proposalId));
            if (proposal) m_buildingCard->setTitle(proposal->summary);
        }
        return;
    }
    if (!m_building.isEmpty() && proposalId.isEmpty() && m_buildingCard) {
        m_justBuilt = m_building;
        m_builtAt = CardPicture::clock();
        // Done: the live answer's card is the proposal's own.
        if (m_live.kerf && m_live.kerf->proposal().isEmpty()) m_live.kerf->setProposal(m_building);
        if (m_live.kerf) m_live.kerf->setProperty("cardKey", QByteArray());
    }
    m_building = proposalId;
    if (m_buildingCard) {
        m_buildingCard->hide();
        m_buildingCard->deleteLater();
        m_buildingCard = nullptr;
    }
    if (!proposalId.isEmpty()) {
        KerfMessage *kerf = ensureLiveKerf();
        auto *card = new ProposalCard(proposalId);
        card->setCompact(true);
        card->setBuilding(true);
        card->setTitle(proposal ? proposal->summary : QString());
        if (m_pictureSource) card->setPicture(m_pictureSource(proposalId));
        card->picture()->setOnStage(proposal && onStage(*proposal));
        connect(card, &ProposalCard::pictureRequested, this, [this, proposalId] { emit pictureRequested(proposalId); });
        moves::fadeIn(card);
        kerf->addCard(card);
        m_buildingCard = card;
        scheduleFollow();
    }
    scheduleRebuild();
}

void ChatPanel::setFootage(const QList<FootageTile> &tiles) {
    if (tiles.isEmpty()) {
        if (m_footage) m_footage->hide();
        syncContent();
        return;
    }
    if (!m_footage) m_footage = new FootageSheet(m_content);
    m_footage->setTiles(tiles);
    m_footage->show();
    rebuild();
    if (m_follow) m_list->verticalScrollBar()->setValue(m_list->verticalScrollBar()->maximum());
}

void ChatPanel::syncContent() {
    // The scroll area sizes its content on a posted layout request; size it
    // now, so the scroll range is the content's before scrolling.
    QCoreApplication::sendPostedEvents(m_content, QEvent::LayoutRequest);
    m_content->layout()->activate();
    const int width = m_list->viewport()->width();
    const int height = std::max(m_list->viewport()->height(),
                                m_content->hasHeightForWidth() ? m_content->heightForWidth(width) : m_content->sizeHint().height());
    if (m_content->size() != QSize(width, height)) m_content->resize(width, height);
}

void ChatPanel::scheduleFollow() {
    if (!m_follow) return;
    QTimer::singleShot(0, this, [this] {
        syncContent();
        m_list->verticalScrollBar()->setValue(m_list->verticalScrollBar()->maximum());
        m_follow = true;
    });
}

// ---- the composer -------------------------------------------------------------------

void ChatPanel::updateComposer(bool busy) { m_composer->setBusy(busy); }

void ChatPanel::updateSuggestions() {
    m_suggestionList.clear();
    // Kerf's next steps.
    for (const QJsonValue &value : std::as_const(m_steps)) {
        const QJsonObject step = value.toObject();
        const QString label = value.isString() ? value.toString() : step.value(QStringLiteral("label")).toString();
        const QString request = value.isString() ? value.toString() : step.value(QStringLiteral("request")).toString();
        if (!label.isEmpty()) m_suggestionList << Suggestion{label, request.isEmpty() ? label : request, false};
    }
    // Each failing check row.
    for (const QJsonValue &value : std::as_const(m_checkRows)) {
        const QJsonObject row = value.toObject();
        if (row.value(QStringLiteral("passed")).toBool(true)) continue;
        const QString name = row.value(QStringLiteral("name")).toString();
        if (name.isEmpty()) continue;
        const QString lowered = name.left(1).toLower() + name.mid(1);
        QString request = QStringLiteral("Fix the %1").arg(lowered);
        const QJsonValue at = row.value(QStringLiteral("at"));
        if (at.isDouble()) request += QStringLiteral(" at %1").arg(chat::timecode(at.toDouble(), m_fps));
        m_suggestionList << Suggestion{QStringLiteral("Fix the %1").arg(lowered), request, false};
    }
    m_suggestionList << Suggestion{QStringLiteral("Run QC on the whole sequence"), QStringLiteral("qc"), true};
    QStringList labels;
    for (const Suggestion &suggestion : std::as_const(m_suggestionList)) labels << suggestion.label;
    m_composer->setSuggestions(labels);
}

QScrollArea *ChatPanel::scroll() const { return m_list; }
QFrame *ChatPanel::composer() const { return m_composer; }
QTextEdit *ChatPanel::input() const { return m_composer->input(); }
QPushButton *ChatPanel::sendButton() const { return m_composer->sendButton(); }
QPushButton *ChatPanel::stopButton() const { return m_composer->stopButton(); }

// ---- the menu ------------------------------------------------------------------------

QMenu *ChatPanel::buildMoreMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("chatMoreMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    ui::styleMenu(menu);
    const bool kept = m_conversations && !m_conversations->currentId().isEmpty();
    const bool archived = kept && m_conversations->current().summary.archived;
    QAction *rename = menu->addAction(QStringLiteral("Rename"));
    rename->setObjectName(QStringLiteral("chatRename"));
    rename->setEnabled(kept && m_connected);
    connect(rename, &QAction::triggered, this, &ChatPanel::startRename);
    QAction *archive = menu->addAction(archived ? QStringLiteral("Unarchive") : QStringLiteral("Archive"));
    archive->setObjectName(QStringLiteral("chatArchive"));
    archive->setEnabled(kept && m_connected);
    connect(archive, &QAction::triggered, this, [this, archived] {
        emit conversationAction(QStringLiteral("archive"), archived ? QStringLiteral("false") : QStringLiteral("true"));
    });
    QAction *remove = menu->addAction(QStringLiteral("Delete…"));
    remove->setObjectName(QStringLiteral("chatDelete"));
    remove->setEnabled(kept && m_connected && !m_live.active);
    connect(remove, &QAction::triggered, this, [this] { emit conversationAction(QStringLiteral("delete"), QString()); });
    menu->addSeparator();
    QAction *settings = menu->addAction(QStringLiteral("Agent settings…"));
    ui::setMenuKey(settings, modelLabel(m_currentModel));
    connect(settings, &QAction::triggered, this, [this] { emit moreAction(QStringLiteral("settings"), QString()); });
    if (m_models.size() > 1 && m_modelsChoosable) {
        QMenu *models = menu->addMenu(QStringLiteral("Model"));
        ui::styleMenu(models);
        for (const QString &model : std::as_const(m_models)) {
            QAction *action = models->addAction(modelLabel(model));
            action->setCheckable(true);
            action->setChecked(model == m_currentModel);
            connect(action, &QAction::triggered, this, [this, model] { emit moreAction(QStringLiteral("model"), model); });
        }
    }
    // A request goes with the playhead's time while a picture is open; here
    // it is taken off, and the composer then says so (its stamp).
    if (m_composer->stampOffered()) {
        QAction *stamp = menu->addAction(QStringLiteral("Send playhead time"));
        stamp->setObjectName(QStringLiteral("chatStamp"));
        stamp->setCheckable(true);
        stamp->setChecked(m_composer->stamped());
        connect(stamp, &QAction::triggered, m_composer, &Composer::setStamped);
    }
    connect(menu->addAction(QStringLiteral("Activity")), &QAction::triggered, this,
            [this] { emit moreAction(QStringLiteral("activity"), QString()); });
    return menu;
}

// ---- events ---------------------------------------------------------------------------

void ChatPanel::paintEvent(QPaintEvent *) {
    QPainter p(this);
    const QRectF area(rect());
    p.setPen(Qt::NoPen);
    // Clear over the window's room, whose light reaches behind it. On its own it
    // fills its place.
    if (!parentWidget() || isWindow()) {
        p.setBrush(color(tokens::chrome));
        p.drawRect(area);
    }
    // The header's hairline, while the panel has a header of its own.
    if (!m_headerShown) return;
    p.setBrush(color(tokens::line));
    p.drawRect(QRectF(0, kHeaderHeight - tokens::layout::seam, width(), tokens::layout::seam));
}

void ChatPanel::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    // With the room to itself the conversation keeps a column to read in,
    // the list and the composer alike, in the middle of whatever width it has:
    // as the window's regions move, what is said goes with them.
    const int column = tokens::layout::conversationColumn;
    const int inset = std::max(0, (width() - column) / 2);
    const QMargins list = m_listLayout->contentsMargins();
    if (list.left() != kSidePadding + inset)
        m_listLayout->setContentsMargins(kSidePadding + inset, list.top(), kSidePadding + inset, list.bottom());
    m_composer->setContentsMargins(inset, 0, inset, 0);
    // Laid out now, at this width, not a frame behind it: the list's posted
    // layout would come after this frame is drawn.
    syncContent();
    updateHeader();
}

bool ChatPanel::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_title && event->type() == QEvent::MouseButtonDblClick) {
        startRename();
        return true;
    }
    if (watched == m_titleEdit && event->type() == QEvent::KeyPress) {
        if (static_cast<QKeyEvent *>(event)->key() == Qt::Key_Escape) {
            m_titleEdit->hide();
            m_title->show();
            emit titleChanged();
            return true;
        }
    }
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
