#include "project_sidebar.h"

#include "design_tokens.h"
#include "icons.h"
#include "stage_motion.h"

#include <QDir>
#include <QFileInfo>
#include <QFontMetrics>
#include <QHBoxLayout>
#include <QLabel>
#include <QMenu>
#include <QPaintEvent>
#include <QPainter>
#include <QPalette>
#include <QVBoxLayout>

namespace kerf {

using namespace tokens;

namespace {

constexpr int kRowHeight = 30;
constexpr int kPadding = space::s8;
constexpr int kIcon = 16;

QString cleanPath(const QString &path) { return path.isEmpty() ? QString() : QDir::cleanPath(QFileInfo(path).absoluteFilePath()); }

} // namespace

SidebarRow::SidebarRow(const QString &icon, const QString &text, int indent, QWidget *parent)
    : ui::KitButton(parent), m_icon(icon), m_indent(indent) {
    setText(text);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setFixedHeight(kRowHeight);
    setFocusPolicy(Qt::TabFocus);
    setAccessibleName(text);
}

void SidebarRow::setTrailing(const QString &text, const QColor &color) {
    if (text == m_trailing && color == m_trailingColor) return;
    m_trailing = text;
    m_trailingColor = color;
    update();
}

void SidebarRow::setCurrent(bool current) {
    if (current == m_current) return;
    m_current = current;
    update();
}

QSize SidebarRow::sizeHint() const { return {layout::sidebar, kRowHeight}; }

void SidebarRow::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    QColor fill = tokens::color(m_current ? on : raised);
    if (!m_current) fill.setAlphaF(float(hoverProgress()));
    p.setPen(Qt::NoPen);
    p.setBrush(fill);
    p.drawRoundedRect(r, radius::control, radius::control);

    const QColor ink_ = tokens::color(m_current || isHovered() ? ink : ink2);
    qreal x = kPadding + m_indent;
    if (!m_icon.isEmpty()) {
        icons::paint(p, m_icon, QRectF(x, 0, kIcon, height()), ink_, kIcon);
        x += kIcon + space::s8;
    }
    qreal right = width() - kPadding;
    if (!m_trailing.isEmpty()) {
        const int w = ui::snapWidth(meta, m_trailing);
        p.setFont(tokens::font(meta));
        p.setPen(m_trailingColor);
        p.drawText(QRectF(right - w, 0, w + 1, height()), Qt::AlignRight | Qt::AlignVCenter, m_trailing);
        right -= w + space::s8;
    }
    p.setFont(tokens::font(body));
    p.setPen(ink_);
    const QString shown = QFontMetrics(p.font()).elidedText(text(), Qt::ElideRight, int(right - x));
    p.drawText(QRectF(x, 0, right - x, height()), Qt::AlignLeft | Qt::AlignVCenter, shown);
    if (focusVisible()) ui::paintFocusRing(p, r, radius::control);
}

SidebarAccount::SidebarAccount(QWidget *parent) : ui::KitButton(parent) {
    setObjectName(QStringLiteral("sidebarAccount"));
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setFixedHeight(layout::sidebarFoot);
    setFocusPolicy(Qt::TabFocus);
}

void SidebarAccount::setAccount(const QString &title, const QString &subtitle) {
    if (title == m_title && subtitle == m_subtitle) return;
    m_title = title;
    m_subtitle = subtitle;
    setAccessibleName(subtitle.isEmpty() ? title : QStringLiteral("%1, %2").arg(title, subtitle));
    update();
}

void SidebarAccount::setOpen(bool open) {
    if (open == m_open) return;
    m_open = open;
    update();
}

QSize SidebarAccount::sizeHint() const { return {layout::sidebar, layout::sidebarFoot}; }

void SidebarAccount::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    QColor fill = tokens::color(raised);
    if (!m_open) fill.setAlphaF(float(hoverProgress()));
    p.setPen(Qt::NoPen);
    p.setBrush(fill);
    p.drawRoundedRect(r, radius::card, radius::card);

    // The disc, with the name's first letter.
    constexpr qreal disc = 28;
    const QRectF avatar(kPadding, (height() - disc) / 2.0, disc, disc);
    p.setBrush(tokens::color(on));
    p.drawEllipse(avatar);
    p.setFont(tokens::font(metaStrong));
    p.setPen(tokens::color(ink));
    p.drawText(avatar, Qt::AlignCenter, m_title.left(1).toUpper());

    const qreal right = width() - kPadding - kIcon;
    icons::paint(p, QStringLiteral("g-updown"), QRectF(right, 0, kIcon, height()), tokens::color(ink3), kIcon);
    const qreal x = avatar.right() + space::s8 + space::s2;
    const qreal room = right - space::s8 - x;
    const bool two = !m_subtitle.isEmpty();
    const qreal top = two ? (height() - 2 * meta.line) / 2.0 : (height() - body.line) / 2.0;
    p.setFont(tokens::font(two ? controlStrong : bodyName));
    p.setPen(tokens::color(ink));
    p.drawText(QRectF(x, top, room, two ? meta.line : body.line), Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetrics(p.font()).elidedText(m_title, Qt::ElideRight, int(room)));
    if (two) {
        p.setFont(tokens::font(meta));
        p.setPen(tokens::color(ink3));
        p.drawText(QRectF(x, top + meta.line, room, meta.line), Qt::AlignLeft | Qt::AlignVCenter,
                   QFontMetrics(p.font()).elidedText(m_subtitle, Qt::ElideRight, int(room)));
    }
    if (focusVisible()) ui::paintFocusRing(p, r, radius::card);
}

ProjectSidebar::ProjectSidebar(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("projectSidebar"));
    setAccessibleName(QStringLiteral("Folders and conversations"));
    m_outer = new QVBoxLayout(this);
    m_outer->setContentsMargins(0, 0, 0, 0);
    m_outer->setSpacing(0);

    m_top = new QWidget;
    m_top->setFixedHeight(space::s8);
    m_outer->addWidget(m_top);

    auto *actions = new QVBoxLayout;
    actions->setContentsMargins(space::s8, 0, space::s8, 0);
    actions->setSpacing(space::s2);
    m_newConversation = new SidebarRow(QStringLiteral("g-plus"), QStringLiteral("New conversation"));
    m_newConversation->setObjectName(QStringLiteral("sidebarNewConversation"));
    connect(m_newConversation, &QAbstractButton::clicked, this, &ProjectSidebar::newConversationRequested);
    actions->addWidget(m_newConversation);
    m_outer->addLayout(actions);

    // Projects, and at its end the one way to another: a new one, or a folder.
    auto *head = new QWidget;
    head->setObjectName(QStringLiteral("sidebarHead"));
    auto *headRow = new QHBoxLayout(head);
    headRow->setContentsMargins(space::s16, space::s12, space::s8, space::s4);
    headRow->setSpacing(space::s4);
    m_heading = new QLabel(QStringLiteral("Projects"));
    m_heading->setObjectName(QStringLiteral("sidebarHeading"));
    m_heading->setFont(tokens::font(meta));
    {
        QPalette palette = m_heading->palette();
        palette.setColor(QPalette::WindowText, tokens::color(ink3));
        m_heading->setPalette(palette);
    }
    headRow->addWidget(m_heading, 1);
    m_add = new ui::IconButton(QStringLiteral("g-plus"), ui::IconButton::Size::S24);
    m_add->setObjectName(QStringLiteral("sidebarAdd"));
    m_add->setAccessibleName(QStringLiteral("New project or open folder"));
    m_add->setTip(QStringLiteral("New project or open folder"));
    m_add->setCheckable(false);
    m_add->setFocusPolicy(Qt::TabFocus);
    m_addMenu = new QMenu(this);
    m_addMenu->setObjectName(QStringLiteral("sidebarAddMenu"));
    ui::styleMenu(m_addMenu);
    QAction *newProject = m_addMenu->addAction(QStringLiteral("New Project…"));
    newProject->setObjectName(QStringLiteral("sidebarNewProject"));
    connect(newProject, &QAction::triggered, this, &ProjectSidebar::newProjectRequested);
    QAction *openFolder = m_addMenu->addAction(QStringLiteral("Open Folder…"));
    openFolder->setObjectName(QStringLiteral("sidebarOpenFolder"));
    connect(openFolder, &QAction::triggered, this, &ProjectSidebar::openFolderRequested);
    connect(m_add, &QAbstractButton::clicked, this, [this] { ui::popupMenu(m_addMenu, m_add); });
    headRow->addWidget(m_add);
    m_outer->addWidget(head);

    m_scroll = new ui::FadeScrollArea;
    m_scroll->setObjectName(QStringLiteral("sidebarScroll"));
    m_scroll->setWidgetResizable(true);
    m_scroll->setFrameShape(QFrame::NoFrame);
    m_scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_scroll->setFadeColor(tokens::color(well));
    m_scroll->setStyleSheet(QStringLiteral("QScrollArea#sidebarScroll { background: transparent; border: 0; }"));
    m_scroll->viewport()->setAutoFillBackground(false);
    auto *content = new QWidget;
    content->setAttribute(Qt::WA_TranslucentBackground);
    m_list = new QVBoxLayout(content);
    m_list->setContentsMargins(space::s8, 0, space::s8, space::s12);
    m_list->setSpacing(space::s2);
    m_list->addStretch(1);
    m_scroll->setWidget(content);
    m_outer->addWidget(m_scroll, 1);

    auto *foot = new QVBoxLayout;
    foot->setContentsMargins(space::s8, 0, space::s8, space::s8);
    m_account = new SidebarAccount;
    connect(m_account, &QAbstractButton::clicked, this, &ProjectSidebar::accountRequested);
    m_account->hide();
    foot->addWidget(m_account);
    m_outer->addLayout(foot);
    m_newConversation->setVisible(false);
}

SidebarRow *ProjectSidebar::newConversationRow() const {
    return m_conversationsShown && !m_current.isEmpty() ? m_newConversation : nullptr;
}

void ProjectSidebar::setAccount(const QString &title, const QString &subtitle) {
    m_account->setAccount(title, subtitle);
    m_account->setVisible(!title.isEmpty());
}

void ProjectSidebar::setProjects(const QStringList &paths, const QString &current) {
    QStringList kept;
    const QString chosen = cleanPath(current);
    for (const QString &path : paths) {
        const QString clean = cleanPath(path);
        if (clean.isEmpty() || kept.contains(clean)) continue;
        if (clean == chosen || QFileInfo(clean).isDir()) kept << clean;
    }
    if (!chosen.isEmpty() && !kept.contains(chosen)) kept.prepend(chosen);
    if (kept == m_projects && chosen == m_current) return;
    m_projects = kept;
    m_current = chosen;
    rebuild();
}

void ProjectSidebar::setConversations(const QList<Conversation> &conversations, const QString &currentId) {
    if (conversations == m_conversations && currentId == m_currentConversation) return;
    const bool sameRows = conversations == m_conversations;
    m_conversations = conversations;
    m_currentConversation = currentId;
    if (sameRows) {
        for (const Conversation &conversation : std::as_const(m_conversations))
            if (SidebarRow *row = conversationRow(conversation.id)) row->setCurrent(conversation.id == currentId);
        return;
    }
    rebuild();
}

void ProjectSidebar::setConversationsShown(bool shown) {
    if (shown == m_conversationsShown) return;
    m_conversationsShown = shown;
    rebuild();
}

void ProjectSidebar::setWorking(bool working) {
    if (working == m_working) return;
    m_working = working;
    if (SidebarRow *row = projectRow(m_current))
        row->setTrailing(working ? QStringLiteral("Working") : QString(), tokens::color(accentInk));
}

void ProjectSidebar::setTopInset(int inset) { m_top->setFixedHeight(inset); }

SidebarRow *ProjectSidebar::projectRow(const QString &path) const {
    const int index = int(m_rowKeys.indexOf(QStringLiteral("project:") + cleanPath(path)));
    return index < 0 ? nullptr : m_rows.at(index);
}

SidebarRow *ProjectSidebar::conversationRow(const QString &id) const {
    const int index = int(m_rowKeys.indexOf(QStringLiteral("conversation:") + id));
    return index < 0 ? nullptr : m_rows.at(index);
}

void ProjectSidebar::rebuild() {
    for (SidebarRow *row : std::as_const(m_rows)) {
        m_list->removeWidget(row);
        row->hide();
        row->deleteLater();
    }
    const QStringList before = m_rowKeys;
    const bool first = before.isEmpty();
    m_rows.clear();
    m_rowKeys.clear();
    m_newConversation->setVisible(m_conversationsShown && !m_current.isEmpty());
    const auto add = [this, before, first](SidebarRow *row, const QString &key) {
        m_list->insertWidget(int(m_rows.size()), row);
        m_rows << row;
        m_rowKeys << key;
        // A new conversation arrives; the rest were here all along.
        if (!first && !before.contains(key)) moves::fadeIn(row);
    };
    constexpr int kChildIndent = space::s16 + space::s8;
    for (const QString &path : std::as_const(m_projects)) {
        auto *row = new SidebarRow(QStringLiteral("g-folder"), QFileInfo(path).fileName());
        row->setObjectName(QStringLiteral("sidebarProject"));
        ui::setTip(row, path);
        const bool chosen = path == m_current;
        row->setCurrent(chosen && (!m_conversationsShown || m_currentConversation.isEmpty()));
        if (chosen && m_working) row->setTrailing(QStringLiteral("Working"), tokens::color(accentInk));
        connect(row, &QAbstractButton::clicked, this, [this, path] { emit projectChosen(path); });
        add(row, QStringLiteral("project:") + path);
        if (!chosen || !m_conversationsShown) continue;
        for (const Conversation &conversation : std::as_const(m_conversations)) {
            auto *child = new SidebarRow(QString(), conversation.title, kChildIndent);
            child->setObjectName(QStringLiteral("sidebarConversation"));
            ui::setTip(child, conversation.title);
            child->setCurrent(conversation.id == m_currentConversation);
            if (conversation.state == State::Working) child->setTrailing(QStringLiteral("Working"), tokens::color(accentInk));
            else if (conversation.state == State::Review) child->setTrailing(QStringLiteral("Review"), tokens::color(accentInk));
            else if (conversation.state == State::Settled) child->setTrailing(QStringLiteral("Done"), tokens::color(ink3));
            const QString id = conversation.id;
            connect(child, &QAbstractButton::clicked, this, [this, id] { emit conversationChosen(id); });
            add(child, QStringLiteral("conversation:") + id);
        }
    }
}

void ProjectSidebar::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    p.fillRect(event->rect(), tokens::color(well));
}

} // namespace kerf
