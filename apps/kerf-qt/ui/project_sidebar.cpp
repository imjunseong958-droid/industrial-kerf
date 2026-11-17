#include "project_sidebar.h"

#include "design_tokens.h"
#include "icons.h"
#include "stage_motion.h"

#include <QAction>
#include <QApplication>
#include <QClipboard>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QFontMetrics>
#include <QHBoxLayout>
#include <QTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QLocale>
#include <QMenu>
#include <QMessageBox>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPalette>
#include <QPushButton>
#include <QSettings>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>
#include <functional>

namespace kerf {

using namespace tokens;

namespace {

// From one row's top to the next; its fill keeps a point clear above and below,
// so rows stand apart with nothing between them for a guide to break on.
constexpr int kRowHeight = 32;
constexpr int kRowInset = 1;
constexpr int kPadding = space::s8;
constexpr int kIcon = 16;
constexpr int kChevron = 12;
constexpr int kChevronGap = space::s4;
constexpr int kBadge = 7;
constexpr int kDotIcon = 12;
constexpr int kAction = 20;
// A conversation's title starts under its folder's icon, with the rest of the
// row to itself.
constexpr int kNoteHeight = 28;
// A folder's picture, in its icon's place and as large.
constexpr int kThumb = kIcon;
// What is in a folder starts under its name, past its icon.
constexpr int kNested = kIcon + space::s8;
// Where a project keeps a small picture of itself, once it has had one.
const QString kPosterFile = QStringLiteral(".kerf/cache/poster.jpg");
// Between one unfolded folder's rows and the next folder's name.
constexpr int kGroupGap = space::s8;
// What the folders' fold is kept under: a map of path to folded.
const QString kFoldedKey = QStringLiteral("sidebar/folded");
// How often the times at the rows' ends are read again.
constexpr int kClockMs = 60 * 1000;

QString cleanPath(const QString &path) { return path.isEmpty() ? QString() : QDir::cleanPath(QFileInfo(path).absoluteFilePath()); }

QString rgba(QRgb color) {
    return QStringLiteral("rgba(%1, %2, %3, %4)").arg(qRed(color)).arg(qGreen(color)).arg(qBlue(color)).arg(qAlpha(color));
}

// The search: the Search row itself, typed into. Its glass and its words
// where the row's were, and nothing round them, neither fill nor outline:
// the caret says where the keys go.
QString searchStyle(const QString &name) {
    return QStringLiteral("QLineEdit#%1 { background: transparent; border: 0; padding: 0 %2px 0 %3px;"
                          " min-height: %4px; max-height: %4px; color: %5;"
                          " selection-background-color: %6; selection-color: %7; }")
        .arg(name)
        .arg(kPadding)
        .arg(kPadding + kIcon + space::s8)
        .arg(kRowHeight)
        .arg(rgba(ink), rgba(accent), rgba(accentText));
}

// A field in the sidebar's own finish: a hairline that turns accent with focus.
QString fieldStyle(const QString &name, int height, QRgb fill, int left) {
    return QStringLiteral("QLineEdit#%1 { background: %2; border: 1px solid %3; border-radius: %4px; padding: 0 6px 0 %10px;"
                          " min-height: %5px; max-height: %5px; color: %6; selection-background-color: %7;"
                          " selection-color: %8; }"
                          "QLineEdit#%1:hover { border-color: %9; }"
                          "QLineEdit#%1:focus { border-color: %7; }")
        .arg(name, rgba(fill), rgba(line))
        .arg(int(radius::control))
        .arg(height - 2)
        .arg(rgba(ink), rgba(accent), rgba(accentText), rgba(line2))
        .arg(left);
}

// A line edit that says when Esc is pressed in it, instead of passing it on.
class EscapeField final : public QLineEdit {
public:
    using QLineEdit::QLineEdit;
    std::function<void()> onEscape;
    // An icon at the start, in ink3: the search's glass.
    QString leadingIcon;

protected:
    void keyPressEvent(QKeyEvent *event) override {
        if (event->key() == Qt::Key_Escape && onEscape) {
            event->accept();
            onEscape();
            return;
        }
        QLineEdit::keyPressEvent(event);
    }
    void paintEvent(QPaintEvent *event) override {
        QLineEdit::paintEvent(event);
        if (leadingIcon.isEmpty()) return;
        // Where a row draws its icon, and as large.
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        icons::paint(p, leadingIcon, QRectF(kPadding, 0, kIcon, height()), tokens::color(ink2), kIcon);
    }
};

void styleField(QLineEdit *field, const QString &name, int height, QRgb fill, int left = space::s6) {
    field->setObjectName(name);
    field->setFont(tokens::font(body));
    field->setStyleSheet(fieldStyle(name, height, fill, left));
    field->setFixedHeight(height);
    field->setAttribute(Qt::WA_MacShowFocusRect, false);
    // Its own hairline turns accent with focus: no second ring round that.
    field->setProperty("ringsOwnFields", true);
    QPalette palette = field->palette();
    palette.setColor(QPalette::PlaceholderText, tokens::color(ink3));
    field->setPalette(palette);
}

// A quiet line where a folder has no rows to show: "No conversations yet".
class SidebarNote final : public QWidget {
public:
    explicit SidebarNote(const QString &text, QWidget *parent = nullptr) : QWidget(parent), m_text(text) {
        setObjectName(QStringLiteral("sidebarNote"));
        setAccessibleName(text);
        setFixedHeight(kNoteHeight);
        setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
        setAttribute(Qt::WA_TransparentForMouseEvents);
    }
    QString text() const { return m_text; }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setFont(tokens::font(control));
        p.setPen(tokens::color(ink3));
        // Under its folder's name, as the folder's conversations are.
        const int x = kPadding + kNested;
        const QRect room(x, 0, width() - x - kPadding, height());
        p.drawText(room, Qt::AlignLeft | Qt::AlignVCenter, QFontMetrics(p.font()).elidedText(m_text, Qt::ElideRight, room.width()));
    }

private:
    QString m_text;
};

// Latest first; those with no time yet come before the rest, in the order
// they came in.
bool latestFirst(const ProjectSidebar::Conversation &a, const ProjectSidebar::Conversation &b) {
    if (a.updated.isValid() != b.updated.isValid()) return !a.updated.isValid();
    return a.updated > b.updated;
}

// How far the light is across the title at `phase` of a pass: it crosses in
// the first part, and the title rests for the rest.
qreal shimmerAt(qreal phase) { return std::clamp(phase / motion::titleCross, 0.0, 1.0); }

} // namespace

SidebarRow::SidebarRow(const QString &icon, const QString &text, int indent, QWidget *parent)
    : ui::KitButton(parent), m_icon(icon), m_indent(indent) {
    setText(text);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setFixedHeight(kRowHeight);
    setFocusPolicy(Qt::TabFocus);
    setAccessibleName(text);
}

void SidebarRow::setCurrent(bool current) {
    if (current == m_current) return;
    m_current = current;
    update();
}

void SidebarRow::setBadge(const QColor &color) {
    if (color == m_badge) return;
    m_badge = color;
    update();
}

void SidebarRow::setShimmer(bool shimmer) {
    if (shimmer == m_shimmer) return;
    m_shimmer = shimmer;
    m_phase = 0.0;
    update();
}

void SidebarRow::setShimmerPhase(qreal phase) {
    if (!m_shimmer || qFuzzyCompare(phase, m_phase)) return;
    m_phase = phase;
    update(textRect());
}

void SidebarRow::setDotIcon(const QString &icon) {
    if (icon == m_dotIcon) return;
    m_dotIcon = icon;
    update();
}

void SidebarRow::setDimmed(bool dimmed) {
    if (dimmed == m_dimmed) return;
    m_dimmed = dimmed;
    update();
}

void SidebarRow::setChevron(Chevron chevron) {
    if (chevron == m_chevron) return;
    m_chevron = chevron;
    update();
}

QRect SidebarRow::chevronRect() const {
    if (m_chevron == Chevron::None) return {};
    if (m_heading) {
        // After the name, with room around it to take a press.
        const QFontMetrics metrics(tokens::font(m_strong ? bodyName : body));
        const int name = std::min(metrics.horizontalAdvance(text()), std::max(0, textRect().width() - kChevron - kChevronGap));
        return QRect(textLeft() + name, 0, kChevronGap * 2 + kChevron, height());
    }
    // The press it takes reaches the row's edge and its full height.
    return QRect(0, 0, kPadding + m_indent + kChevron + kChevronGap, height());
}

void SidebarRow::setStrong(bool strong) {
    if (strong == m_strong) return;
    m_strong = strong;
    update();
}

void SidebarRow::setHeading(bool heading) {
    if (heading == m_heading) return;
    m_heading = heading;
    update();
}

void SidebarRow::setGuide(bool guide) {
    if (guide == m_guide) return;
    m_guide = guide;
    update();
}

void SidebarRow::mousePressEvent(QMouseEvent *event) {
    m_onChevron = event->button() == Qt::LeftButton && chevronRect().contains(event->position().toPoint());
    if (m_onChevron) {
        event->accept();
        return;
    }
    ui::KitButton::mousePressEvent(event);
}

void SidebarRow::mouseReleaseEvent(QMouseEvent *event) {
    if (!m_onChevron) {
        ui::KitButton::mouseReleaseEvent(event);
        return;
    }
    m_onChevron = false;
    event->accept();
    if (chevronRect().contains(event->position().toPoint())) emit chevronPressed();
}

void SidebarRow::setHoverActions(const QList<ui::IconButton *> &actions) {
    for (ui::IconButton *action : std::as_const(m_actions))
        if (!actions.contains(action)) action->deleteLater();
    m_actions = actions;
    for (ui::IconButton *action : std::as_const(m_actions)) {
        action->setParent(this);
        action->setVisible(isHovered() && !m_editing);
    }
    placeActions();
    update();
}

bool SidebarRow::actionsShown() const {
    return !m_actions.isEmpty() && !m_editing && m_actions.first()->isVisibleTo(this);
}

void SidebarRow::setEditing(bool editing) {
    if (editing == m_editing) return;
    m_editing = editing;
    for (ui::IconButton *action : std::as_const(m_actions)) action->setVisible(!editing && isHovered());
    update();
}

int SidebarRow::textLeft() const {
    int x = kPadding + m_indent;
    if (m_chevron != Chevron::None && !m_heading) x += kChevron + kChevronGap;
    if (!m_icon.isEmpty()) x += kIcon + space::s8;
    else if (!m_dotIcon.isEmpty()) x += kDotIcon + space::s6;
    return x;
}

QRect SidebarRow::textRect() const {
    const int x = textLeft();
    int right = width() - kPadding;
    if (actionsShown()) right = m_actions.first()->x() - space::s4;
    else if (m_badge.isValid()) right -= kBadge + space::s8;
    return QRect(x, 0, std::max(0, right - x), height());
}

void SidebarRow::placeActions() {
    int right = width() - space::s4;
    for (int i = int(m_actions.size()) - 1; i >= 0; --i) {
        ui::IconButton *action = m_actions.at(i);
        action->setGeometry(right - kAction, (height() - kAction) / 2, kAction, kAction);
        right -= kAction + space::s2;
    }
}

void SidebarRow::enterEvent(QEnterEvent *event) {
    ui::KitButton::enterEvent(event);
    if (m_actions.isEmpty() || m_editing) return;
    for (ui::IconButton *action : std::as_const(m_actions)) action->show();
    update();
}

void SidebarRow::leaveEvent(QEvent *event) {
    ui::KitButton::leaveEvent(event);
    if (m_actions.isEmpty()) return;
    for (ui::IconButton *action : std::as_const(m_actions)) action->hide();
    update();
}

void SidebarRow::resizeEvent(QResizeEvent *event) {
    ui::KitButton::resizeEvent(event);
    placeActions();
}

QSize SidebarRow::sizeHint() const { return {layout::sidebar, kRowHeight}; }

void SidebarRow::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    // The fill keeps clear of the rows over and under it.
    const QRectF r = QRectF(rect()).adjusted(0, kRowInset, 0, -kRowInset);
    // Its folder's guide, under the fill: a row that is on or under the
    // pointer stands over it.
    if (m_guide) p.fillRect(QRect(kPadding + kChevron / 2, 0, 1, height()), tokens::color(line));
    QColor fill = tokens::color(m_current ? on : raised);
    if (!m_current) fill.setAlphaF(float(hoverProgress()));
    p.setPen(Qt::NoPen);
    p.setBrush(fill);
    p.drawRoundedRect(r, radius::control, radius::control);

    const bool lit = m_current || isHovered();
    if (m_heading) {
        // A folder over its conversations: its picture or its glyph, its name,
        // and the chevron after it. The open folder's name is in ink, medium.
        const QColor name = tokens::color(m_strong || lit ? ink : ink2);
        paintLead(p, kPadding + m_indent, m_strong || lit ? tokens::color(ink2) : tokens::color(ink3));
        if (!m_editing) {
            const QRect room = textRect();
            p.setFont(tokens::font(m_strong ? bodyName : body));
            const QFontMetrics metrics(p.font());
            const QString shown = metrics.elidedText(text(), Qt::ElideRight, std::max(0, room.width() - kChevron - kChevronGap));
            QPen pen(name);
            if (m_shimmer && !ui::reducedMotion()) {
                const qreal wide = std::max(1.0, qreal(metrics.horizontalAdvance(shown)));
                const qreal band = std::max(24.0, wide * 0.35);
                const qreal centre = room.left() - band + shimmerAt(m_phase) * (wide + 2 * band);
                QLinearGradient light(centre - band, 0, centre + band, 0);
                light.setColorAt(0.0, tokens::color(ink3));
                light.setColorAt(0.5, tokens::color(ink));
                light.setColorAt(1.0, tokens::color(ink3));
                pen = QPen(QBrush(light), 1);
            }
            p.setPen(pen);
            p.drawText(room, Qt::AlignLeft | Qt::AlignVCenter, shown);
            if (m_chevron != Chevron::None) {
                const qreal x = room.left() + metrics.horizontalAdvance(shown) + kChevronGap;
                icons::paint(p, m_chevron == Chevron::Folded ? QStringLiteral("g-chevr") : QStringLiteral("g-chev"),
                             QRectF(x, (height() - kChevron) / 2.0, kChevron, kChevron), tokens::color(ink3), kChevron);
            }
        }
        if (focusVisible()) ui::paintFocusRing(p, r, radius::control);
        return;
    }
    const QColor ink_ = tokens::color(m_dimmed ? (lit ? ink2 : ink3) : (lit || m_strong ? ink : ink2));
    qreal x0 = kPadding + m_indent;
    if (m_chevron != Chevron::None) {
        icons::paint(p, m_chevron == Chevron::Folded ? QStringLiteral("g-chevr") : QStringLiteral("g-chev"),
                     QRectF(x0, (height() - kChevron) / 2.0, kChevron, kChevron), tokens::color(lit ? ink2 : ink3), kChevron);
        x0 += kChevron + kChevronGap;
    }
    if (!m_icon.isEmpty() || !m_thumb.isNull())
        paintLead(p, x0, m_strong ? tokens::color(lit ? ink : ink2) : ink_);
    else if (!m_dotIcon.isEmpty())
        icons::paint(p, m_dotIcon, QRectF(x0, (height() - kDotIcon) / 2.0, kDotIcon, kDotIcon), ink_, kDotIcon);
    // The dot at the end, where the actions go while the pointer is over the row.
    if (m_badge.isValid() && !actionsShown()) {
        p.setPen(Qt::NoPen);
        p.setBrush(m_badge);
        p.drawEllipse(QRectF(width() - kPadding - kBadge, (height() - kBadge) / 2.0, kBadge, kBadge));
    }
    if (!m_editing) {
        const QRect room = textRect();
        p.setFont(tokens::font(m_strong ? bodyName : body));
        const QString shown = QFontMetrics(p.font()).elidedText(text(), Qt::ElideRight, room.width());
        QPen pen(ink_);
        if (m_shimmer && !ui::reducedMotion()) {
            // A band of light passes over the title, from its start to its end
            // (MOTION_UI.md, Sidebar activity), over the quieter ink it rests in.
            const qreal wide = std::max(1.0, qreal(QFontMetrics(p.font()).horizontalAdvance(shown)));
            const qreal band = std::max(24.0, wide * 0.35);
            const qreal centre = room.left() - band + shimmerAt(m_phase) * (wide + 2 * band);
            QLinearGradient light(centre - band, 0, centre + band, 0);
            light.setColorAt(0.0, tokens::color(ink3));
            light.setColorAt(0.5, tokens::color(ink));
            light.setColorAt(1.0, tokens::color(ink3));
            pen = QPen(QBrush(light), 1);
        } else if (m_shimmer) {
            pen = QPen(tokens::color(ink));
        }
        p.setPen(pen);
        p.drawText(room, Qt::AlignLeft | Qt::AlignVCenter, shown);
    }
    if (focusVisible()) ui::paintFocusRing(p, r, radius::control);
}

void SidebarRow::setThumb(const QPixmap &thumb) {
    m_thumb = thumb;
    update();
}

void SidebarRow::paintLead(QPainter &p, qreal x, const QColor &color) const {
    if (m_thumb.isNull()) {
        if (!m_icon.isEmpty()) icons::paint(p, m_icon, QRectF(x, 0, kIcon, height()), color, kIcon);
        return;
    }
    // The picture, in the icon's column, its corners rounded and a hairline
    // round it so a dark frame still has an edge.
    const QRectF box(x + (kIcon - kThumb) / 2.0, (height() - kThumb) / 2.0, kThumb, kThumb);
    QPainterPath corners;
    corners.addRoundedRect(box, radius::small, radius::small);
    p.save();
    p.setClipPath(corners);
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    // Filled, cut at its sides or its top and foot as its shape wants.
    const QSizeF size = m_thumb.deviceIndependentSize();
    const qreal scale = std::max(box.width() / size.width(), box.height() / size.height());
    const QSizeF drawn = size * scale;
    p.drawPixmap(QRectF(box.center().x() - drawn.width() / 2.0, box.center().y() - drawn.height() / 2.0, drawn.width(), drawn.height()),
                 m_thumb, QRectF(QPointF(0, 0), QSizeF(m_thumb.size())));
    p.restore();
    ui::paintInsetLine(p, box, radius::small, tokens::color(line2));
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

    // "Name · what it is on ⌄" on one line, as a coding agent's foot reads.
    const qreal x = avatar.right() + space::s8;
    const qreal room = width() - kPadding - x - kChevron - kChevronGap;
    const QFont nameFont = tokens::font(controlStrong);
    const QFont restFont = tokens::font(control);
    const QFontMetrics nameMetrics(nameFont);
    const QFontMetrics restMetrics(restFont);
    const QString name = nameMetrics.elidedText(m_title, Qt::ElideRight, int(room * (m_subtitle.isEmpty() ? 1.0 : 0.6)));
    const qreal nameWide = nameMetrics.horizontalAdvance(name);
    p.setFont(nameFont);
    p.setPen(tokens::color(ink));
    p.drawText(QRectF(x, 0, nameWide + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, name);
    qreal end = x + nameWide;
    if (!m_subtitle.isEmpty()) {
        const QString rest = restMetrics.elidedText(QStringLiteral(" · %1").arg(m_subtitle), Qt::ElideRight, int(room - nameWide));
        p.setFont(restFont);
        p.setPen(tokens::color(ink3));
        p.drawText(QRectF(end, 0, room - nameWide, height()), Qt::AlignLeft | Qt::AlignVCenter, rest);
        end += restMetrics.horizontalAdvance(rest);
    }
    icons::paint(p, m_open ? QStringLiteral("g-chevr") : QStringLiteral("g-chev"),
                 QRectF(end + kChevronGap, (height() - kChevron) / 2.0, kChevron, kChevron), tokens::color(ink3), kChevron);
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
    m_newConversation = new SidebarRow(QStringLiteral("g-plus"), QStringLiteral("New"));
    m_newConversation->setObjectName(QStringLiteral("sidebarNewConversation"));
    m_newConversation->setAccessibleName(QStringLiteral("New conversation"));
    connect(m_newConversation, &QAbstractButton::clicked, this, &ProjectSidebar::newConversationRequested);
    actions->addWidget(m_newConversation);
    m_searchRow = new SidebarRow(QStringLiteral("g-search"), QStringLiteral("Search"));
    m_searchRow->setObjectName(QStringLiteral("sidebarSearchRow"));
    m_searchRow->setAccessibleName(QStringLiteral("Search conversations"));
    connect(m_searchRow, &QAbstractButton::clicked, this, &ProjectSidebar::openSearch);
    actions->addWidget(m_searchRow);

    // The search over the chosen folder's conversations, in the Search row's
    // place while it is open: Esc empties it, and empty, closes it.
    auto *search = new EscapeField;
    search->setObjectName(QStringLiteral("sidebarSearch"));
    search->setFont(tokens::font(body));
    search->setStyleSheet(searchStyle(search->objectName()));
    search->setFixedHeight(kRowHeight);
    search->setAttribute(Qt::WA_MacShowFocusRect, false);
    // No ring round it either (editor_common.cpp rings a field being typed in).
    search->setProperty("ringsOwnFields", true);
    {
        QPalette palette = search->palette();
        palette.setColor(QPalette::PlaceholderText, tokens::color(ink3));
        search->setPalette(palette);
    }
    search->leadingIcon = QStringLiteral("g-search");
    search->setPlaceholderText(QStringLiteral("Search"));
    search->setAccessibleName(QStringLiteral("Search conversations"));
    search->onEscape = [this, search] {
        if (!search->text().isEmpty()) {
            search->clear();
            return;
        }
        search->clearFocus();
        search->hide();
        m_searchRow->setVisible(m_newConversation->isVisible());
    };
    search->installEventFilter(this);
    connect(search, &QLineEdit::textChanged, this, [this](const QString &text) {
        const QString query = text.trimmed();
        if (query == m_query) return;
        m_query = query;
        rebuild(false);
    });
    m_search = search;
    actions->addWidget(m_search);
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

    // A conversation's menu, made ready for the one it opens over.
    m_conversationMenu = new QMenu(this);
    m_conversationMenu->setObjectName(QStringLiteral("sidebarConversationMenu"));
    ui::styleMenu(m_conversationMenu);
    QAction *rename = m_conversationMenu->addAction(QStringLiteral("Rename…"));
    rename->setObjectName(QStringLiteral("sidebarRename"));
    connect(rename, &QAction::triggered, this, [this] { beginRename(m_menuFor); });
    QAction *archiving = m_conversationMenu->addAction(QStringLiteral("Archive"));
    archiving->setObjectName(QStringLiteral("sidebarArchive"));
    connect(archiving, &QAction::triggered, this, [this] {
        const int index = indexOf(m_menuFor);
        if (index >= 0) archive(m_menuFor, !m_conversations.at(index).archived);
    });
    m_conversationMenu->addSeparator();
    QAction *remove = m_conversationMenu->addAction(QStringLiteral("Delete…"));
    remove->setObjectName(QStringLiteral("sidebarDelete"));
    ui::setMenuRole(remove, ui::MenuRole::Warn);
    connect(remove, &QAction::triggered, this, [this] { confirmDelete(m_menuFor); });

    // A folder's menu, made ready for the one it opens over.
    m_projectMenu = new QMenu(this);
    m_projectMenu->setObjectName(QStringLiteral("sidebarProjectMenu"));
    ui::styleMenu(m_projectMenu);
    QAction *newHere = m_projectMenu->addAction(QStringLiteral("New Conversation"));
    newHere->setObjectName(QStringLiteral("sidebarProjectNew"));
    connect(newHere, &QAction::triggered, this, [this] {
        if (m_projectMenuFor == m_current) emit newConversationRequested();
        else emit conversationChosenIn(m_projectMenuFor, QString());
    });
    QAction *openIt = m_projectMenu->addAction(QStringLiteral("Open Project"));
    openIt->setObjectName(QStringLiteral("sidebarProjectOpen"));
    connect(openIt, &QAction::triggered, this, [this] { emit projectChosen(m_projectMenuFor); });
    m_projectMenu->addSeparator();
#if defined(Q_OS_MACOS)
    const QString reveal = QStringLiteral("Show in Finder");
#elif defined(Q_OS_WIN)
    const QString reveal = QStringLiteral("Show in Explorer");
#else
    const QString reveal = QStringLiteral("Show in Files");
#endif
    QAction *show = m_projectMenu->addAction(reveal);
    show->setObjectName(QStringLiteral("sidebarProjectReveal"));
    connect(show, &QAction::triggered, this, [this] { emit projectRevealRequested(m_projectMenuFor); });
    QAction *copy = m_projectMenu->addAction(QStringLiteral("Copy Path"));
    copy->setObjectName(QStringLiteral("sidebarProjectCopyPath"));
    connect(copy, &QAction::triggered, this, [this] { QApplication::clipboard()->setText(QDir::toNativeSeparators(m_projectMenuFor)); });
    m_projectMenu->addSeparator();
    QAction *forget = m_projectMenu->addAction(QStringLiteral("Remove from Sidebar"));
    forget->setObjectName(QStringLiteral("sidebarProjectRemove"));
    connect(forget, &QAction::triggered, this, [this] { emit projectRemoveRequested(m_projectMenuFor); });

    // What was folded and unfolded by hand, as it was left.
    const QVariantMap folded = QSettings().value(kFoldedKey).toMap();
    for (auto it = folded.cbegin(); it != folded.cend(); ++it) m_folded.insert(it.key(), it.value().toBool());

    m_scroll = new ui::FadeScrollArea;
    m_scroll->setObjectName(QStringLiteral("sidebarScroll"));
    m_scroll->setWidgetResizable(true);
    m_scroll->setFrameShape(QFrame::NoFrame);
    m_scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_scroll->setFadeColor(tokens::color(sidebar));
    m_scroll->setStyleSheet(QStringLiteral("QScrollArea#sidebarScroll { background: transparent; border: 0; }"));
    m_scroll->viewport()->setAutoFillBackground(false);
    auto *content = new QWidget;
    content->setAttribute(Qt::WA_TranslucentBackground);
    m_list = new QVBoxLayout(content);
    m_list->setContentsMargins(space::s8, 0, space::s8, space::s12);
    // The rows keep their own point of air, so a folder's guide runs unbroken.
    m_list->setSpacing(0);
    m_list->addStretch(1);
    m_scroll->setWidget(content);
    m_outer->addWidget(m_scroll, 1);

    auto *foot = new QVBoxLayout;
    foot->setContentsMargins(space::s8, 0, space::s8, space::s8);
    auto *footRow = new QHBoxLayout;
    footRow->setContentsMargins(0, 0, 0, 0);
    footRow->setSpacing(space::s4);
    m_account = new SidebarAccount;
    connect(m_account, &QAbstractButton::clicked, this, &ProjectSidebar::accountRequested);
    m_account->hide();
    footRow->addWidget(m_account, 1);
    m_settings = new ui::IconButton(QStringLiteral("g-settings"), ui::IconButton::Size::S28);
    m_settings->setObjectName(QStringLiteral("sidebarSettings"));
    m_settings->setCheckable(false);
    m_settings->setAccessibleName(QStringLiteral("Settings"));
    m_settings->setTip(QStringLiteral("Settings"), ui::keyText(QKeySequence(Qt::CTRL | Qt::Key_Comma)));
    connect(m_settings, &QAbstractButton::clicked, this, &ProjectSidebar::settingsRequested);
    footRow->addWidget(m_settings, 0, Qt::AlignVCenter);
    foot->addLayout(footRow);
    m_outer->addLayout(foot);
    m_newConversation->setVisible(false);
    m_searchRow->setVisible(false);
    m_search->setVisible(false);

    // The light over a working title: only that title is drawn again, 30 times
    // a second, and only while it can be seen (MOTION_UI.md, Sidebar activity).
    m_shimmer.setStartValue(0.0);
    m_shimmer.setEndValue(1.0);
    m_shimmer.setDuration(motion::titleShimmer);
    m_shimmer.setLoopCount(-1);
    connect(&m_shimmer, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        const int step = qRound(value.toReal() * motion::titleShimmerSteps);
        if (step == m_shimmerStep) return;
        m_shimmerStep = step;
        const qreal phase = qreal(step) / motion::titleShimmerSteps;
        for (SidebarRow *row : std::as_const(m_rows))
            if (row->isShimmering()) row->setShimmerPhase(phase);
    });
    m_clock.setInterval(kClockMs);
    connect(&m_clock, &QTimer::timeout, this, &ProjectSidebar::refreshTimes);
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
    if (chosen != m_current) m_archivedOpen = false;
    m_projects = kept;
    m_current = chosen;
    readOthers();
    rebuild();
}

QList<ProjectSidebar::Conversation> ProjectSidebar::readConversations(const QString &project) {
    QList<Conversation> out;
    const QDir folder(QDir(project).filePath(QStringLiteral(".kerf/conversations")));
    for (const QFileInfo &entry : folder.entryInfoList({QStringLiteral("*.json")}, QDir::Files)) {
        QFile file(entry.absoluteFilePath());
        if (!file.open(QIODevice::ReadOnly)) continue;
        const QJsonObject object = QJsonDocument::fromJson(file.readAll()).object();
        Conversation conversation;
        conversation.id = object.value(QStringLiteral("id")).toString();
        // A file that is not a conversation is left out, as the bridge leaves it.
        if (!conversation.id.startsWith(QStringLiteral("c-"))) continue;
        conversation.title = object.value(QStringLiteral("title")).toString();
        if (conversation.title.isEmpty()) conversation.title = QStringLiteral("Conversation");
        conversation.archived = object.value(QStringLiteral("archived")).toBool();
        const QString updated = object.value(QStringLiteral("updated")).toString();
        conversation.updated = QDateTime::fromString(updated, Qt::ISODateWithMs);
        if (!conversation.updated.isValid()) conversation.updated = QDateTime::fromString(updated, Qt::ISODate);
        conversation.count = int(object.value(QStringLiteral("items")).toArray().size());
        out << conversation;
    }
    std::stable_sort(out.begin(), out.end(), latestFirst);
    return out;
}

void ProjectSidebar::readOthers() {
    QHash<QString, QList<Conversation>> read;
    if (m_conversationsShown)
        for (const QString &path : std::as_const(m_projects))
            if (path != m_current && !isFolded(path)) read.insert(path, readConversations(path));
    m_others = read;
}

QList<ProjectSidebar::Conversation> ProjectSidebar::conversationsOf(const QString &path) const {
    const QString clean = cleanPath(path);
    return clean == m_current ? m_conversations : m_others.value(clean);
}

bool ProjectSidebar::isFolded(const QString &path) const {
    const QString clean = cleanPath(path);
    // The open folder shows what is in it until it is folded by hand.
    return m_folded.value(clean, clean != m_current);
}

void ProjectSidebar::setFolded(const QString &path, bool folded) {
    const QString clean = cleanPath(path);
    if (clean.isEmpty() || isFolded(clean) == folded) return;
    m_folded.insert(clean, folded);
    // Kept only for folders still listed, so the map does not grow for ever.
    QVariantMap kept;
    for (auto it = m_folded.cbegin(); it != m_folded.cend(); ++it)
        if (m_projects.contains(it.key())) kept.insert(it.key(), it.value());
    QSettings().setValue(kFoldedKey, kept);
    if (!folded && clean != m_current) m_others.insert(clean, readConversations(clean));
    rebuild(false);
}

void ProjectSidebar::setConversations(const QList<Conversation> &conversations, const QString &currentId) {
    if (conversations == m_conversations && currentId == m_currentConversation) return;
    // What stopped working while you were not looking at it keeps a dot until
    // you do.
    QHash<QString, State> was;
    for (const Conversation &conversation : std::as_const(m_conversations)) was.insert(conversation.id, conversation.state);
    for (const Conversation &conversation : conversations) {
        const State before = was.value(conversation.id, State::Idle);
        const bool stopped = (before == State::Working || before == State::Making) && conversation.state == State::Idle;
        if (stopped && !(watching() && conversation.id == currentId)) m_unseen.insert(conversation.id);
    }
    const bool sameRows = conversations == m_conversations;
    m_conversations = conversations;
    m_currentConversation = currentId;
    if (watching()) m_unseen.remove(currentId);
    if (sameRows) {
        // Which of a long folder's rows show depends on the one that is open:
        // chosen from past the first few, it comes into sight with the rest.
        if (m_conversations.size() > kShownAtFirst && !isFolded(m_current)) {
            rebuild(false);
            return;
        }
        for (const Conversation &conversation : std::as_const(m_conversations))
            if (SidebarRow *row = conversationRow(conversation.id)) {
                row->setCurrent(conversation.id == currentId);
                row->setBadge(badgeOf(conversation));
            }
        return;
    }
    rebuild();
}

bool ProjectSidebar::watching() const { return window() && window()->isActiveWindow(); }

void ProjectSidebar::markSeen() {
    if (!watching() || !m_unseen.remove(m_currentConversation)) return;
    const int index = indexOf(m_currentConversation);
    if (SidebarRow *row = conversationRow(m_currentConversation); row && index >= 0) row->setBadge(badgeOf(m_conversations.at(index)));
}

void ProjectSidebar::changeEvent(QEvent *event) {
    QWidget::changeEvent(event);
    if (event->type() == QEvent::ActivationChange) markSeen();
}

QColor ProjectSidebar::badgeOf(const Conversation &conversation) const {
    switch (conversation.state) {
    case State::Review: return tokens::color(activity::review);
    case State::Failed: return tokens::color(activity::failed);
    case State::Settled: return tokens::color(activity::settled);
    case State::Idle: return m_unseen.contains(conversation.id) ? tokens::color(activity::settled) : QColor();
    case State::Working:
    case State::Making: break;
    }
    return {};
}

QString ProjectSidebar::stateOf(const Conversation &conversation) const {
    switch (conversation.state) {
    case State::Working: return QStringLiteral("Working");
    case State::Making: return QStringLiteral("Making");
    case State::Review: return QStringLiteral("Review");
    case State::Failed: return QStringLiteral("Failed");
    case State::Settled: return QStringLiteral("Done");
    case State::Idle: return m_unseen.contains(conversation.id) ? QStringLiteral("Done") : QString();
    }
    return {};
}

void ProjectSidebar::setConversationsShown(bool shown) {
    if (shown == m_conversationsShown) return;
    m_conversationsShown = shown;
    readOthers();
    rebuild();
}

void ProjectSidebar::setWorking(bool working) {
    if (working == m_working) return;
    m_working = working;
    // A folded folder shows that Kerf works in it on its name.
    if (SidebarRow *row = projectRow(m_current)) row->setShimmer(working && (!m_conversationsShown || isFolded(m_current)));
    updateShimmer();
}

void ProjectSidebar::setTopInset(int inset) { m_top->setFixedHeight(inset); }

void ProjectSidebar::setArchivedOpen(bool open) {
    if (open == m_archivedOpen) return;
    m_archivedOpen = open;
    rebuild(false, open);
}

SidebarRow *ProjectSidebar::projectRow(const QString &path) const {
    const int index = int(m_rowKeys.indexOf(QStringLiteral("project:") + cleanPath(path)));
    return index < 0 ? nullptr : m_rows.at(index);
}

SidebarRow *ProjectSidebar::conversationRow(const QString &id) const {
    const int index = int(m_rowKeys.indexOf(QStringLiteral("conversation:") + id));
    return index < 0 ? nullptr : m_rows.at(index);
}

SidebarRow *ProjectSidebar::archivedRow() const {
    const int index = int(m_rowKeys.indexOf(QStringLiteral("archived")));
    return index < 0 ? nullptr : m_rows.at(index);
}

SidebarRow *ProjectSidebar::moreRow(const QString &path) const {
    const int index = int(m_rowKeys.indexOf(QStringLiteral("more:") + cleanPath(path)));
    return index < 0 ? nullptr : m_rows.at(index);
}

QMenu *ProjectSidebar::projectMenu(const QString &path) {
    const QString clean = cleanPath(path);
    if (!m_projects.contains(clean)) return nullptr;
    m_projectMenuFor = clean;
    const bool open = clean == m_current;
    // The open project is open already, and stays on the list while it is.
    if (auto *action = m_projectMenu->findChild<QAction *>(QStringLiteral("sidebarProjectOpen"))) action->setVisible(!open);
    if (auto *action = m_projectMenu->findChild<QAction *>(QStringLiteral("sidebarProjectRemove"))) action->setEnabled(!open);
    return m_projectMenu;
}

int ProjectSidebar::indexOf(const QString &id) const {
    for (int i = 0; i < m_conversations.size(); ++i)
        if (m_conversations.at(i).id == id) return i;
    return -1;
}

QDateTime ProjectSidebar::now() const { return m_fixedNow.isValid() ? m_fixedNow : QDateTime::currentDateTime(); }

QString ProjectSidebar::relativeTime(const QDateTime &when, const QDateTime &now) {
    if (!when.isValid() || !now.isValid()) return {};
    const qint64 seconds = std::max<qint64>(0, when.secsTo(now));
    if (seconds < 60) return QStringLiteral("now");
    if (seconds < 60 * 60) return QStringLiteral("%1m").arg(seconds / 60);
    if (seconds < 24 * 60 * 60) return QStringLiteral("%1h").arg(seconds / (60 * 60));
    if (seconds < 7 * 24 * 60 * 60) return QStringLiteral("%1d").arg(seconds / (24 * 60 * 60));
    const QDate day = when.toLocalTime().date();
    const QLocale c = QLocale::c();
    return day.year() == now.toLocalTime().date().year() ? c.toString(day, QStringLiteral("MMM d"))
                                                         : c.toString(day, QStringLiteral("MMM yyyy"));
}

QMenu *ProjectSidebar::conversationMenu(const QString &id) {
    const int index = indexOf(id);
    if (index < 0) return nullptr;
    m_menuFor = id;
    const bool archived = m_conversations.at(index).archived;
    if (auto *action = m_conversationMenu->findChild<QAction *>(QStringLiteral("sidebarArchive")))
        action->setText(archived ? QStringLiteral("Unarchive") : QStringLiteral("Archive"));
    return m_conversationMenu;
}

void ProjectSidebar::showConversationMenu(const QString &id, const QPoint &globalPos, bool fromKeyboard) {
    QMenu *menu = conversationMenu(id);
    if (!menu) return;
    SidebarRow *row = conversationRow(id);
    if (fromKeyboard && row) ui::popupMenu(menu, row);
    else menu->popup(globalPos);
}

void ProjectSidebar::archive(const QString &id, bool archived) {
    const int index = indexOf(id);
    if (index < 0 || m_conversations.at(index).archived == archived) return;
    m_conversations[index].archived = archived;
    rebuild(false);
    emit archiveRequested(id, archived);
}

void ProjectSidebar::confirmDelete(const QString &id) {
    int index = indexOf(id);
    if (index < 0) return;
    const QString title = m_conversations.at(index).title;
    QMessageBox box(this);
    box.setObjectName(QStringLiteral("sidebarDeleteConfirm"));
    box.setIcon(QMessageBox::Warning);
    box.setWindowTitle(QStringLiteral("Delete Conversation"));
    box.setText(QStringLiteral("Delete “%1”?").arg(title));
    box.setInformativeText(QStringLiteral("This can't be undone."));
    QPushButton *remove = box.addButton(QStringLiteral("Delete"), QMessageBox::DestructiveRole);
    remove->setObjectName(QStringLiteral("sidebarDeleteConfirmButton"));
    QPushButton *keep = box.addButton(QMessageBox::Cancel);
    box.setDefaultButton(keep);
    box.setEscapeButton(keep);
    box.exec();
    if (box.clickedButton() != remove) return;
    // The list may have changed while the question was asked.
    index = indexOf(id);
    if (index >= 0) m_conversations.removeAt(index);
    rebuild(false);
    emit deleteRequested(id);
}

void ProjectSidebar::beginRename(const QString &id) {
    SidebarRow *row = conversationRow(id);
    const int index = indexOf(id);
    if (!row || index < 0) return;
    if (m_renameField) finishRename(true);
    auto *field = new EscapeField(row);
    // Its text stays where the title was, and its edge keeps clear of the dot.
    constexpr int inset = space::s4 - 1;
    styleField(field, QStringLiteral("sidebarRenameField"), kRowHeight - 2 * space::s4 + space::s2, card, inset);
    field->setAccessibleName(QStringLiteral("Conversation title"));
    field->setText(m_conversations.at(index).title);
    const int left = row->textLeft() - inset - 1;
    field->setGeometry(left, (row->height() - field->height()) / 2, row->width() - left - space::s4, field->height());
    field->onEscape = [this] { finishRename(false); };
    connect(field, &QLineEdit::editingFinished, this, [this] { finishRename(true); });
    m_renameField = field;
    m_renaming = id;
    row->setEditing(true);
    field->show();
    field->setFocus(Qt::OtherFocusReason);
    field->selectAll();
}

void ProjectSidebar::finishRename(bool keep) {
    if (!m_renameField) return;
    QLineEdit *field = m_renameField;
    const QString id = m_renaming;
    const QString title = field->text().simplified();
    const bool hadFocus = field->hasFocus();
    m_renameField = nullptr;
    m_renaming.clear();
    field->disconnect(this);
    field->hide();
    field->deleteLater();
    SidebarRow *row = conversationRow(id);
    if (row) {
        row->setEditing(false);
        if (hadFocus) row->setFocus(Qt::OtherFocusReason);
    }
    const int index = indexOf(id);
    if (!keep || index < 0 || title.isEmpty() || title == m_conversations.at(index).title) return;
    m_conversations[index].title = title;
    if (row) {
        row->setText(title);
        row->setAccessibleName(title);
        ui::setTip(row, title);
    }
    emit renameRequested(id, title);
}

SidebarRow *ProjectSidebar::makeConversationRow(const Conversation &conversation, bool dimmed, const QString &project) {
    // Under its folder's name, past the folder's icon.
    auto *row = new SidebarRow(QString(), conversation.title, kNested);
    row->setObjectName(QStringLiteral("sidebarConversation"));
    row->setDimmed(dimmed);
    // When it last changed is a hover away.
    const QString when = relativeTime(conversation.updated, now());
    ui::setTip(row, when.isEmpty() ? conversation.title : QStringLiteral("%1 · %2").arg(conversation.title, when));
    const bool here = project == m_current;
    if (!here) {
        // Another folder's: it is opened, with its project, and that is all.
        row->setProperty("projectPath", project);
        row->setAccessibleDescription(QStringLiteral("In %1").arg(QFileInfo(project).fileName()));
        connect(row, &QAbstractButton::clicked, this, [this, project, id = conversation.id] { emit conversationChosenIn(project, id); });
        return row;
    }
    row->setProperty("conversationId", conversation.id);
    row->setCurrent(conversation.id == m_currentConversation);
    const bool working = conversation.state == State::Working || conversation.state == State::Making;
    row->setShimmer(working);
    if (working) row->setShimmerPhase(qreal(std::max(0, m_shimmerStep)) / motion::titleShimmerSteps);
    row->setBadge(badgeOf(conversation));
    QStringList about;
    if (const QString state = stateOf(conversation); !state.isEmpty()) about << state;
    if (conversation.archived) about << QStringLiteral("Archived");
    if (conversation.count > 0)
        about << (conversation.count == 1 ? QStringLiteral("1 message") : QStringLiteral("%1 messages").arg(conversation.count));
    row->setAccessibleDescription(about.join(QStringLiteral(", ")));

    // One way to all it can be asked, as its right-click.
    const QString id = conversation.id;
    auto *more = new ui::IconButton(QStringLiteral("g-more"), ui::IconButton::Size::S20);
    more->setObjectName(QStringLiteral("sidebarConversationMenuButton"));
    more->setCheckable(false);
    more->setFocusPolicy(Qt::NoFocus);
    more->setAccessibleName(QStringLiteral("More for %1").arg(conversation.title));
    more->setTip(QStringLiteral("More"));
    connect(more, &QAbstractButton::clicked, this, [this, id, more] {
        if (QMenu *menu = conversationMenu(id)) ui::popupMenu(menu, more);
    });
    row->setHoverActions({more});

    row->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(row, &QWidget::customContextMenuRequested, this,
            [this, row, id](const QPoint &pos) { showConversationMenu(id, row->mapToGlobal(pos), false); });
    row->installEventFilter(this);
    connect(row, &QAbstractButton::clicked, this, [this, id] { emit conversationChosen(id); });
    return row;
}

void ProjectSidebar::openSearch() {
    m_searchRow->hide();
    m_search->show();
    m_search->setFocus(Qt::OtherFocusReason);
}

bool ProjectSidebar::eventFilter(QObject *watched, QEvent *event) {
    // The field, left empty, gives way to the Search row again.
    if (watched == m_search && event->type() == QEvent::FocusOut && m_search->text().trimmed().isEmpty()) {
        QTimer::singleShot(0, this, [this] {
            if (m_search->hasFocus() || !m_search->text().trimmed().isEmpty()) return;
            m_search->hide();
            m_searchRow->setVisible(m_newConversation->isVisible());
        });
    }
    if (event->type() == QEvent::KeyPress) {
        auto *row = qobject_cast<SidebarRow *>(watched);
        // On a folder: Right unfolds it, Left folds it, the menu key opens its menu.
        if (row && row->objectName() == QStringLiteral("sidebarProject") && row->chevron() != SidebarRow::Chevron::None) {
            const QString path = row->property("projectPath").toString();
            const auto *key = static_cast<QKeyEvent *>(event);
            if (key->key() == Qt::Key_Right || key->key() == Qt::Key_Left) {
                setFolded(path, key->key() == Qt::Key_Left);
                if (SidebarRow *again = projectRow(path)) again->setFocus(Qt::OtherFocusReason);
                return true;
            }
            if ((key->key() == Qt::Key_F10 && (key->modifiers() & Qt::ShiftModifier)) || key->key() == Qt::Key_Menu) {
                if (QMenu *menu = projectMenu(path)) ui::popupMenu(menu, row);
                return true;
            }
        }
        const QString id = row ? row->property("conversationId").toString() : QString();
        if (!id.isEmpty()) {
            const auto *key = static_cast<QKeyEvent *>(event);
            const bool shift = key->modifiers() & Qt::ShiftModifier;
            if (key->key() == Qt::Key_F2) {
                beginRename(id);
                return true;
            }
            if (key->key() == Qt::Key_Delete || key->key() == Qt::Key_Backspace) {
                confirmDelete(id);
                return true;
            }
            if ((key->key() == Qt::Key_F10 && shift) || key->key() == Qt::Key_Menu) {
                showConversationMenu(id, row->mapToGlobal(QPoint(row->textLeft(), row->height())), true);
                return true;
            }
        }
    }
    return QWidget::eventFilter(watched, event);
}

SidebarRow *ProjectSidebar::makeProjectRow(const QString &path) {
    // A folder: its picture once it has one, else the folder's glyph, and its
    // name. With its conversations under it, its chevron follows its name.
    auto *row = new SidebarRow(QStringLiteral("g-folder"), QFileInfo(path).fileName());
    row->setObjectName(QStringLiteral("sidebarProject"));
    row->setProperty("projectPath", path);
    row->setThumb(posterOf(path));
    ui::setTip(row, path);
    const bool chosen = path == m_current;
    row->setHeading(m_conversationsShown);
    // The start window's folders are chosen among; the editor's head their
    // conversations, and the open one's name is a step brighter.
    row->setCurrent(chosen && !m_conversationsShown);
    row->setStrong(!m_conversationsShown || chosen);
    if (!m_conversationsShown) {
        row->setShimmer(chosen && m_working);
        // The start window: a folder is chosen to work in.
        connect(row, &QAbstractButton::clicked, this, [this, path] { emit projectChosen(path); });
        return row;
    }
    const bool folded = isFolded(path);
    // Folded, its name shows that Kerf works in it; unfolded, the conversation's does.
    row->setShimmer(chosen && m_working && folded);
    row->setChevron(folded ? SidebarRow::Chevron::Folded : SidebarRow::Chevron::Unfolded);
    row->setAccessibleDescription(folded ? QStringLiteral("Folded") : QStringLiteral("Unfolded"));
    // Its name, like its chevron, folds and unfolds it: what is in it opens it.
    const auto toggle = [this, path] { setFolded(path, !isFolded(path)); };
    connect(row, &SidebarRow::chevronPressed, this, toggle);
    connect(row, &QAbstractButton::clicked, this, toggle);

    auto *add = new ui::IconButton(QStringLiteral("g-plus"), ui::IconButton::Size::S20);
    add->setObjectName(QStringLiteral("sidebarProjectNewButton"));
    add->setCheckable(false);
    add->setFocusPolicy(Qt::NoFocus);
    add->setAccessibleName(QStringLiteral("New conversation in %1").arg(row->text()));
    add->setTip(chosen ? QStringLiteral("New conversation") : QStringLiteral("Open with a new conversation"));
    connect(add, &QAbstractButton::clicked, this, [this, path, chosen] {
        if (chosen) emit newConversationRequested();
        else emit conversationChosenIn(path, QString());
    });
    auto *more = new ui::IconButton(QStringLiteral("g-more"), ui::IconButton::Size::S20);
    more->setObjectName(QStringLiteral("sidebarProjectMenuButton"));
    more->setCheckable(false);
    more->setFocusPolicy(Qt::NoFocus);
    more->setAccessibleName(QStringLiteral("More for %1").arg(row->text()));
    more->setTip(QStringLiteral("More"));
    connect(more, &QAbstractButton::clicked, this, [this, path, more] {
        if (QMenu *menu = projectMenu(path)) ui::popupMenu(menu, more);
    });
    row->setHoverActions({add, more});

    row->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(row, &QWidget::customContextMenuRequested, this, [this, row, path](const QPoint &pos) {
        if (QMenu *menu = projectMenu(path)) menu->popup(row->mapToGlobal(pos));
    });
    row->installEventFilter(this);
    return row;
}

void ProjectSidebar::rebuild(bool arrivals, bool unfolding) {
    // A title being typed is carried over to the row that takes its place.
    QString renaming;
    QString typed;
    if (m_renameField) {
        renaming = m_renaming;
        typed = m_renameField->text();
        m_renameField->disconnect(this);
        m_renameField->hide();
        m_renameField->deleteLater();
        m_renameField = nullptr;
        m_renaming.clear();
    }
    for (SidebarRow *row : std::as_const(m_rows)) {
        m_list->removeWidget(row);
        row->hide();
        row->deleteLater();
    }
    for (QWidget *extra : std::as_const(m_extras)) {
        m_list->removeWidget(extra);
        extra->hide();
        extra->deleteLater();
    }
    const QStringList before = m_rowKeys;
    const bool first = before.isEmpty();
    m_rows.clear();
    m_rowKeys.clear();
    m_extras.clear();
    m_noteTexts.clear();
    const bool unfolded = m_conversationsShown && !m_current.isEmpty();
    m_newConversation->setVisible(unfolded);
    // Search is a row until it is opened; a query keeps the field open.
    const bool searching = unfolded && (!m_query.isEmpty() || !m_search->isHidden());
    m_search->setVisible(searching);
    m_searchRow->setVisible(unfolded && !searching);
    int at = 0;
    const auto add = [this, before, first, arrivals, &at](SidebarRow *row, const QString &key) {
        m_list->insertWidget(at++, row);
        m_rows << row;
        m_rowKeys << key;
        // A new conversation arrives; the rest were here all along.
        if (arrivals && !first && !before.contains(key)) moves::fadeIn(row);
    };
    const auto note = [this, &at](const QString &text) {
        auto *line = new SidebarNote(text);
        m_list->insertWidget(at++, line);
        m_extras << line;
        m_noteTexts << text;
    };
    bool afterRows = false;
    for (const QString &path : std::as_const(m_projects)) {
        // A folder's name stands off from the rows of the one over it.
        if (afterRows) {
            auto *gap = new QWidget;
            gap->setFixedHeight(kGroupGap);
            gap->setAttribute(Qt::WA_TransparentForMouseEvents);
            m_list->insertWidget(at++, gap);
            m_extras << gap;
        }
        add(makeProjectRow(path), QStringLiteral("project:") + path);
        afterRows = m_conversationsShown && !isFolded(path);
        if (!afterRows) continue;
        const bool open = path == m_current;

        QList<Conversation> live;
        QList<Conversation> archived;
        for (const Conversation &conversation : conversationsOf(path)) {
            if (!m_query.isEmpty() && !conversation.title.contains(m_query, Qt::CaseInsensitive)) continue;
            (conversation.archived ? archived : live) << conversation;
        }
        std::stable_sort(live.begin(), live.end(), latestFirst);
        // Another folder's archived conversations stay out of the way: they
        // are its own to bring back, once it is open.
        if (!open) archived.clear();
        if (live.isEmpty() && archived.isEmpty()) {
            note(m_query.isEmpty() ? QStringLiteral("No conversations yet") : QStringLiteral("No conversations match"));
            continue;
        }
        // The first few, unless the rest were asked for, are searched, or hold
        // the conversation that is open.
        qsizetype shown = live.size();
        if (live.size() > kShownAtFirst && m_query.isEmpty() && !m_showAll.contains(path)) {
            shown = kShownAtFirst;
            for (qsizetype index = kShownAtFirst; index < live.size(); ++index)
                if (open && live.at(index).id == m_currentConversation) shown = live.size();
        }
        for (qsizetype index = 0; index < shown; ++index)
            add(makeConversationRow(live.at(index), false, path), QStringLiteral("conversation:") + (open ? QString() : path + QLatin1Char('#')) + live.at(index).id);
        if (live.size() > kShownAtFirst && m_query.isEmpty()) {
            const bool all = shown == live.size();
            if (!all || m_showAll.contains(path)) {
                auto *more = new SidebarRow(QString(), all ? QStringLiteral("Show less") : QStringLiteral("Show %1 more").arg(live.size() - shown), kNested);
                more->setObjectName(QStringLiteral("sidebarMore"));
                more->setDimmed(true);
                connect(more, &QAbstractButton::clicked, this, [this, path, all] {
                    if (all) m_showAll.remove(path);
                    else m_showAll.insert(path);
                    rebuild(false);
                });
                add(more, QStringLiteral("more:") + path);
            }
        }
        if (archived.isEmpty()) continue;

        // The archived ones, out of the way at the end, folded until asked for.
        const QString count = QStringLiteral("Archived · %1").arg(archived.size());
        // Its chevron in the icons' column, its words under the folder's name.
        auto *fold = new SidebarRow(QString(), count, kNested - kDotIcon - space::s6);
        fold->setObjectName(QStringLiteral("sidebarArchived"));
        fold->setDotIcon(m_archivedOpen ? QStringLiteral("g-chev") : QStringLiteral("g-chevr"));
        fold->setAccessibleDescription(m_archivedOpen ? QStringLiteral("Unfolded") : QStringLiteral("Folded"));
        fold->setDimmed(true);
        connect(fold, &QAbstractButton::clicked, this, [this] { setArchivedOpen(!m_archivedOpen); });
        add(fold, QStringLiteral("archived"));
        if (!m_archivedOpen) continue;
        std::stable_sort(archived.begin(), archived.end(), latestFirst);
        int unfoldedSoFar = 0;
        for (const Conversation &conversation : std::as_const(archived)) {
            SidebarRow *child = makeConversationRow(conversation, true, path);
            add(child, QStringLiteral("conversation:") + conversation.id);
            if (unfolding) moves::fadeIn(child, unfoldedSoFar++ * motion::unfoldStagger);
        }
    }
    if (!renaming.isEmpty() && conversationRow(renaming)) {
        beginRename(renaming);
        if (m_renameField) m_renameField->setText(typed);
    }
    updateShimmer();
}

QPixmap ProjectSidebar::posterOf(const QString &path) const {
    const QFileInfo file(QDir(path).filePath(kPosterFile));
    if (!file.isFile()) return {};
    const auto kept = m_posters.constFind(path);
    if (kept != m_posters.constEnd() && kept->first == file.lastModified()) return kept->second;
    QPixmap picture(file.absoluteFilePath());
    m_posters.insert(path, {file.lastModified(), picture});
    return picture;
}

QString ProjectSidebar::posterPath(const QString &project) { return QDir(project).filePath(kPosterFile); }

void ProjectSidebar::refreshPosters() {
    for (SidebarRow *row : std::as_const(m_rows))
        if (row->objectName() == QStringLiteral("sidebarProject")) row->setThumb(posterOf(row->property("projectPath").toString()));
}

void ProjectSidebar::refreshTimes() {
    // What the other folders keep may have changed under them: read again.
    const QHash<QString, QList<Conversation>> were = m_others;
    readOthers();
    if (m_others != were) {
        rebuild(false);
        return;
    }
    // When each last changed, in its tip.
    const QDateTime clock = now();
    const auto tip = [&clock](SidebarRow *row, const Conversation &conversation) {
        const QString when = relativeTime(conversation.updated, clock);
        ui::setTip(row, when.isEmpty() ? conversation.title : QStringLiteral("%1 · %2").arg(conversation.title, when));
    };
    for (auto it = m_others.cbegin(); it != m_others.cend(); ++it)
        for (const Conversation &conversation : it.value()) {
            const int index = int(m_rowKeys.indexOf(QStringLiteral("conversation:") + it.key() + QLatin1Char('#') + conversation.id));
            if (index >= 0) tip(m_rows.at(index), conversation);
        }
    for (const Conversation &conversation : std::as_const(m_conversations))
        if (SidebarRow *row = conversationRow(conversation.id)) tip(row, conversation);
}

void ProjectSidebar::updateShimmer() {
    bool any = false;
    for (const SidebarRow *row : std::as_const(m_rows)) any = any || row->isShimmering();
    const bool run = any && isVisible() && !ui::reducedMotion();
    if (run) {
        if (m_shimmer.state() != QAbstractAnimation::Running) m_shimmer.start();
        return;
    }
    m_shimmer.stop();
    m_shimmerStep = -1;
}

void ProjectSidebar::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    refreshTimes();
    m_clock.start();
    updateShimmer();
}

void ProjectSidebar::hideEvent(QHideEvent *event) {
    QWidget::hideEvent(event);
    m_clock.stop();
    updateShimmer();
}

void ProjectSidebar::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    p.fillRect(event->rect(), tokens::color(sidebar));
}

} // namespace kerf
