#include "proposal_card.h"

#include "comment_row.h"
#include "components.h"
#include "design_tokens.h"
#include "icons.h"

#include <QEvent>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QLabel>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QSignalBlocker>
#include <QVBoxLayout>

#include <algorithm>
#include <array>
#include <cmath>
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
    label->setFixedHeight(style.line);
    return label;
}

void setInk(QLabel *label, QRgb ink) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
}

// The script diff (.diff): chrome, radius 4, an inset line, pad 4 8, mono 11/16.
class DiffView final : public QLabel {
public:
    DiffView() {
        setObjectName(QStringLiteral("proposalScript"));
        setFont(tokens::font(monoSmall));
        setTextFormat(Qt::RichText);
        setContentsMargins(8, 4, 8, 4);
        setWordWrap(false);
        setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        setTextInteractionFlags(Qt::TextSelectableByMouse);
        QPalette palette = this->palette();
        palette.setColor(QPalette::WindowText, color(ink2));
        setPalette(palette);
    }

protected:
    void paintEvent(QPaintEvent *event) override {
        {
            QPainter p(this);
            p.setRenderHint(QPainter::Antialiasing);
            p.setPen(Qt::NoPen);
            p.setBrush(color(chrome));
            p.drawRoundedRect(QRectF(rect()), radius::small, radius::small);
            ui::paintInsetLine(p, QRectF(rect()), radius::small, color(line));
        }
        QLabel::paintEvent(event);
    }
};

// The "Waiting for the editor" chip: 24 tall, radius 6, on, 11 ink2.
class WaitChip final : public QLabel {
public:
    WaitChip() {
        setText(QStringLiteral("Waiting for the editor"));
        setFont(tokens::font(meta));
        setContentsMargins(8, 0, 8, 0);
        setFixedHeight(24);
        setFixedWidth(ui::snapWidth(QFontMetricsF(tokens::font(meta)), text()) + 16);
        QPalette palette = this->palette();
        palette.setColor(QPalette::WindowText, color(ink2));
        setPalette(palette);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        p.setPen(Qt::NoPen);
        p.setBrush(color(on));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
        const QFont face = tokens::font(meta);
        const QFontMetricsF metrics(face);
        p.setFont(face);
        p.setPen(color(ink2));
        p.drawText(QPointF(8, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    }
};

// A change row (.chg): 24 tall, name UI 12 ink (elided), target mono 12
// ink2, shift mono 11 ink3 right-aligned in 40, gap 12.
class ChangeRow final : public QWidget {
public:
    explicit ChangeRow(const ProposalCard::Change &change) : m_change(change) {
        setObjectName(QStringLiteral("proposalChange"));
        setFixedHeight(24);
        setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
        ui::setTip(this, QStringLiteral("%1 %2 %3").arg(change.name, change.target, change.shift).trimmed());
        setAccessibleName(QStringLiteral("%1 %2 %3").arg(change.name, change.target, change.shift).trimmed());
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        const QFont name = tokens::font(control);
        const QFont target = tokens::font(timecode);
        const QFont shift = tokens::font(monoSmall);
        const int shiftWidth = 40;
        const int targetWidth = ui::snapWidth(QFontMetricsF(target), m_change.target);
        const int nameWidth = std::max(0, width() - shiftWidth - targetWidth - 24);
        const auto draw = [&p, this](const QFont &face, QRgb ink, const QRectF &box, const QString &text, Qt::Alignment align) {
            const QFontMetricsF metrics(face);
            p.setFont(face);
            p.setPen(color(ink));
            const QString shown = metrics.elidedText(text, Qt::ElideRight, box.width());
            const qreal baseline = (height() - metrics.height()) / 2 + metrics.ascent();
            const qreal w = metrics.horizontalAdvance(shown);
            const qreal x = align & Qt::AlignRight ? box.right() - w : box.left();
            p.drawText(QPointF(x, baseline), shown);
        };
        draw(name, ink, QRectF(0, 0, nameWidth, height()), m_change.name, Qt::AlignLeft);
        draw(target, ink2, QRectF(nameWidth + 12, 0, targetWidth, height()), m_change.target, Qt::AlignLeft);
        draw(shift, ink3, QRectF(width() - shiftWidth, 0, shiftWidth, height()), m_change.shift, Qt::AlignRight);
    }

private:
    ProposalCard::Change m_change;
};

QWidget *spaced(QWidget *child, int top) {
    auto *box = new QWidget;
    auto *layout = new QVBoxLayout(box);
    layout->setContentsMargins(0, top, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(child);
    return box;
}

} // namespace

// ------------------------------------------------------------------ VectorscopeChip

VectorscopeChip::VectorscopeChip(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("vectorscopeChip"));
    setFixedSize(kSize, kSize);
    setAccessibleName(QStringLiteral("Vectorscope, before and after"));
}

scope::VectorscopeGeometry VectorscopeChip::geometry() {
    scope::VectorscopeGeometry g;
    g.centre = QPointF(kSize / 2.0, kSize / 2.0);
    g.radius = kRadius;
    g.scale = kRadius / 0.5;
    return g;
}

scope::Density VectorscopeChip::bin(const QImage &frame) {
    if (frame.isNull()) return {};
    return scope::vectorscope(scope::scopeSource(frame, QSize(160, 90)), QSize(kSize, kSize), geometry(), 1.0);
}

void VectorscopeChip::setFrames(const QImage &before, const QImage &after) {
    m_before = bin(before);
    m_after = bin(after);
    update();
}

void VectorscopeChip::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    p.setBrush(color(well));
    p.drawRoundedRect(QRectF(rect()), radius::small, radius::small);
    const scope::VectorscopeGeometry g = geometry();
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(color(track2), 1));
    p.drawEllipse(g.centre, g.radius, g.radius);
    // The six 75% colour-bar targets.
    static const std::array<std::array<int, 3>, 6> bars{{{191, 0, 0}, {191, 0, 191}, {0, 0, 191}, {0, 191, 191}, {0, 191, 0}, {191, 191, 0}}};
    for (const auto &bar : bars) {
        const QPointF at = g.point(scope::chroma(bar[0], bar[1], bar[2]), 1.0);
        p.drawRect(QRectF(at.x() - 2, at.y() - 2, 4, 4));
    }
    p.setPen(Qt::NoPen);
    const auto dots = [&p](const scope::Density &density, QColor ink) {
        p.setBrush(ink);
        for (int y = 0; y < density.height; ++y)
            for (int x = 0; x < density.width; ++x)
                if (density.at(x, y)) p.drawEllipse(QPointF(x + 0.5, y + 0.5), 0.8, 0.8);
    };
    QColor before = color(ink3);
    before.setAlphaF(0.5);
    dots(m_before, before);
    dots(m_after, color(ink));
}

// ------------------------------------------------------------------ VersionButton

VersionButton::VersionButton(QWidget *parent) : QAbstractButton(parent) {
    setFont(tokens::font(monoSmall));
    setFixedHeight(20);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    ui::setTip(this, QStringLiteral("Proposal versions"));
}

QSize VersionButton::sizeHint() const {
    return QSize(6 + ui::snapWidth(QFontMetricsF(font()), text()) + 2 + 14 + 2, 20);
}

void VersionButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const bool hover = underMouse() || isDown();
    if (hover) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(hover2));
        p.drawRoundedRect(QRectF(rect()), radius::small, radius::small);
    }
    const QColor ink = hover ? color(tokens::ink) : color(ink3);
    const QFontMetricsF metrics(font());
    p.setFont(font());
    p.setPen(ink);
    p.drawText(QPointF(6, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    icons::paint(p, QStringLiteral("g-chev"), QRectF(width() - 2 - 14, (height() - 14) / 2.0, 14, 14), ink, 14);
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::small);
}

// ------------------------------------------------------------------ ProposalCard

ProposalCard::ProposalCard(const QString &proposalId, QWidget *parent) : QFrame(parent), m_id(proposalId) {
    setObjectName(QStringLiteral("proposalCard"));
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    m_layout = new QVBoxLayout(this);
    m_layout->setContentsMargins(12, 12, 12, 12);
    m_layout->setSpacing(0);

    // The pinned header: 16 tall, gap 6, 8 above the title.
    m_pinHeader = new QWidget;
    m_pinHeader->setFixedHeight(16 + 8);
    auto *pinLayout = new QHBoxLayout(m_pinHeader);
    pinLayout->setContentsMargins(0, 0, 0, 8);
    pinLayout->setSpacing(6);
    auto *pin = new QLabel;
    pin->setFixedSize(12, 16);
    pin->setPixmap(icons::pixmap(QStringLiteral("g-pin"), color(ink3), 12, devicePixelRatioF()));
    pin->setAlignment(Qt::AlignCenter);
    pinLayout->addWidget(pin);
    m_pinText = makeLabel(meta, ink3);
    m_pinText->setText(QStringLiteral("Pinned proposal"));
    pinLayout->addWidget(m_pinText, 1);
    m_layout->addWidget(m_pinHeader);
    m_pinHeader->hide();

    // The title: click opens or closes the card.
    m_title = new QLabel;
    m_title->setObjectName(QStringLiteral("proposalTitle"));
    m_title->setFont(tokens::font(bodyName));
    m_title->setWordWrap(true);
    m_title->setTextFormat(Qt::PlainText);
    m_title->setCursor(Qt::PointingHandCursor);
    m_title->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    setInk(m_title, ink);
    m_title->installEventFilter(this);
    m_layout->addWidget(m_title);

    // The second line: N changes, then the version button at the right.
    m_second = new QWidget;
    m_second->setFixedHeight(20);
    auto *second = new QHBoxLayout(m_second);
    second->setContentsMargins(0, 0, 0, 0);
    second->setSpacing(8);
    m_changesLink = new ChatExpander;
    m_changesLink->setText(QStringLiteral("0 changes"));
    connect(m_changesLink, &QAbstractButton::clicked, this, [this] {
        setOpen(!m_open);
        emit openToggled(m_open);
    });
    second->addWidget(m_changesLink);
    second->addStretch(1);
    m_version = new VersionButton;
    m_version->hide();
    connect(m_version, &QAbstractButton::clicked, this, &ProposalCard::showVersionMenu);
    second->addWidget(m_version);
    m_layout->addWidget(m_second);

    // The changes, 4 under it.
    m_changes = new QWidget;
    m_changesLayout = new QVBoxLayout(m_changes);
    m_changesLayout->setContentsMargins(0, 4, 0, 0);
    m_changesLayout->setSpacing(0);
    m_colourBox = new QWidget;
    m_colourBox->setObjectName(QStringLiteral("colourPreview"));
    auto *colour = new QHBoxLayout(m_colourBox);
    colour->setContentsMargins(0, 4, 0, 4);
    colour->setSpacing(12);
    m_chip = new VectorscopeChip;
    colour->addWidget(m_chip, 0, Qt::AlignTop);
    auto *colourText = new QVBoxLayout;
    colourText->setContentsMargins(0, 0, 0, 0);
    colourText->setSpacing(4);
    m_colourTitle = makeLabel(control, ink);
    m_colourParams = makeLabel(monoSmall, ink2);
    m_colourLegend = makeLabel(meta, ink3);
    m_colourLegend->setText(QStringLiteral("Vectorscope: before grey, after white"));
    for (QLabel *label : {m_colourTitle, m_colourParams, m_colourLegend}) {
        label->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
        colourText->addWidget(label);
    }
    colourText->addStretch(1);
    colour->addLayout(colourText, 1);
    m_colourBox->hide();
    m_changesLayout->addWidget(m_colourBox);
    m_layout->addWidget(m_changes);

    // The script diff, 8 under the changes.
    m_diff = new DiffView;
    QWidget *diffBox = spaced(m_diff, 8);
    diffBox->setObjectName(QStringLiteral("diffBox"));
    diffBox->hide();
    m_layout->addWidget(diffBox);

    // The footer: 24 tall, 8 under, gap 8.
    m_footer = new QWidget;
    m_footer->setObjectName(QStringLiteral("proposalFooter"));
    m_footer->setFixedHeight(24 + 8);
    auto *footer = new QHBoxLayout(m_footer);
    footer->setContentsMargins(0, 8, 0, 0);
    footer->setSpacing(8);
    m_watch = new ui::Button(QString(), ui::Button::Kind::Secondary);
    m_watch->setObjectName(QStringLiteral("proposalWatch"));
    m_watch->setTip(QStringLiteral("Play only what this proposal changes"));
    // Across the card, 8 under the changes.
    m_watch->setMinimumWidth(0);
    m_watch->setMaximumWidth(QWIDGETSIZE_MAX);
    m_watch->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    connect(m_watch, &QAbstractButton::clicked, this, &ProposalCard::watchRequested);
    m_watchBox = spaced(m_watch, 8);
    m_watchBox->setObjectName(QStringLiteral("proposalWatchBox"));
    m_watchBox->hide();
    m_layout->addWidget(m_watchBox);
    m_preview = new ui::CheckBox(QStringLiteral("Preview"));
    m_preview->setTip(QStringLiteral("Preview the proposal on the program"));
    connect(m_preview, &QAbstractButton::toggled, this, &ProposalCard::previewToggled);
    footer->addWidget(m_preview);
    m_script = new ui::GhostButton(QStringLiteral("Script"));
    m_script->setHorizontalPadding(6);
    m_script->setCheckable(true);
    m_script->setTip(QStringLiteral("Show the script"));
    connect(m_script, &QAbstractButton::toggled, this, [this](bool shown) {
        setScriptShown(shown);
        emit scriptToggled(shown);
    });
    footer->addWidget(m_script);
    footer->addStretch(1);
    m_wait = new WaitChip;
    m_wait->hide();
    footer->addWidget(m_wait);
    m_reject = new ui::Button(QStringLiteral("Reject"), ui::Button::Kind::Secondary);
    m_reject->setTip(QStringLiteral("Reject"), QStringLiteral("Esc"));
    connect(m_reject, &QAbstractButton::clicked, this, [this] { emit rejectRequested(shownId()); });
    footer->addWidget(m_reject);
    m_accept = new ui::Button(QStringLiteral("Accept"), ui::Button::Kind::Secondary);
    m_acceptKey = ui::keyText(QKeySequence(Qt::CTRL | Qt::Key_Return));
    m_accept->setTip(m_acceptTip, m_acceptKey);
    connect(m_accept, &QAbstractButton::clicked, this, [this] { emit acceptRequested(shownId()); });
    footer->addWidget(m_accept);
    m_layout->addWidget(m_footer);

    // The settled line.
    m_settledLabel = makeLabel(control, accentInk);
    m_settledLabel->setFixedHeight(20);
    m_settledLabel->hide();
    m_layout->addWidget(m_settledLabel);

    updateParts();
}

void ProposalCard::setPinned(bool pinned, bool blocking) {
    m_pinned = pinned;
    m_pinText->setText(blocking ? QStringLiteral("Pinned proposal · blocking first") : QStringLiteral("Pinned proposal"));
    updateParts();
}

void ProposalCard::setTitle(const QString &title) {
    m_title->setText(title);
    setAccessibleName(title);
}

void ProposalCard::setChanges(const QList<Change> &changes) {
    for (QWidget *row : std::as_const(m_changeRows)) {
        m_changesLayout->removeWidget(row);
        delete row;
    }
    m_changeRows.clear();
    int index = 0;
    for (const Change &change : changes) {
        auto *row = new ChangeRow(change);
        m_changesLayout->insertWidget(index++, row);
        m_changeRows.append(row);
    }
    const int count = int(changes.size());
    m_changesLink->setText(count == 1 ? QStringLiteral("1 change") : QStringLiteral("%1 changes").arg(count));
    m_changesLink->updateGeometry();
}

void ProposalCard::setVersions(const QList<VersionItem> &versions, const QString &shownId) {
    m_versions = versions;
    m_shown = shownId;
    int shownVersion = 0;
    for (const VersionItem &item : versions)
        if (item.proposalId == shownId) shownVersion = item.version;
    if (shownVersion == 0 && !versions.isEmpty()) shownVersion = versions.constLast().version;
    m_version->setText(QStringLiteral("v%1").arg(shownVersion));
    m_version->setFixedSize(m_version->sizeHint());
    m_version->setVisible(versions.size() > 1);
}

void ProposalCard::showVersionMenu() {
    auto *menu = new QMenu(this);
    menu->setAttribute(Qt::WA_DeleteOnClose);
    menu->setObjectName(QStringLiteral("versionsMenu"));
    for (auto it = m_versions.crbegin(); it != m_versions.crend(); ++it) {
        const VersionItem item = *it;
        QAction *action = menu->addAction(item.label.isEmpty() ? QStringLiteral("v%1").arg(item.version) : item.label);
        action->setCheckable(true);
        action->setChecked(item.proposalId == shownId());
        action->setData(item.proposalId);
        connect(action, &QAction::triggered, this, [this, id = item.proposalId] {
            if (id == shownId()) return;
            setVersions(m_versions, id);
            emit versionChosen(id);
        });
    }
    // Oldest first, as the design lists them.
    QList<QAction *> actions = menu->actions();
    std::reverse(actions.begin(), actions.end());
    for (QAction *action : actions) menu->removeAction(action);
    menu->addActions(actions);
    ui::popupMenu(menu, m_version);
}

void ProposalCard::setColour(const Colour &colour) {
    m_colour = colour;
    m_colourTitle->setText(colour.title);
    m_colourParams->setText(colour.params);
    m_colourParams->setVisible(!colour.params.isEmpty());
    m_colourBox->setVisible(!colour.clip.isEmpty());
}

void ProposalCard::clearColour() { setColour(Colour{}); }

void ProposalCard::setColourFrames(const QImage &before, const QImage &after) { m_chip->setFrames(before, after); }

void ProposalCard::setDiff(const QStringList &lines) {
    m_diffLines = lines;
    QString html;
    for (const QString &line : lines) {
        const bool header = line.startsWith(QLatin1String("+++")) || line.startsWith(QLatin1String("---"));
        const QRgb ink = !header && line.startsWith(QLatin1Char('+'))   ? tokens::pos
                         : !header && line.startsWith(QLatin1Char('-')) ? tokens::neg
                                                                         : tokens::ink2;
        if (!html.isEmpty()) html += QStringLiteral("<br>");
        html += QStringLiteral("<span style=\"color:%1; white-space:pre;\">%2</span>")
                    .arg(color(ink).name(QColor::HexRgb), line.toHtmlEscaped());
    }
    m_diff->setText(QStringLiteral("<div style=\"line-height:16px;\">%1</div>").arg(html));
    m_script->setEnabled(!lines.isEmpty());
}

void ProposalCard::setOpen(bool open) {
    m_open = open;
    m_changesLink->setChecked(open);
    updateParts();
}

void ProposalCard::setScriptShown(bool shown) {
    {
        const QSignalBlocker blocker(m_script);
        m_script->setChecked(shown);
    }
    m_diff->parentWidget()->setVisible(shown && m_open && !m_settled);
    updateGeometry();
}

bool ProposalCard::scriptShown() const { return m_script->isChecked(); }

void ProposalCard::setWatchLength(double seconds) {
    m_hasWatch = seconds > 0.0;
    m_watch->setText(QStringLiteral("Watch what changed · %1 s").arg(int(std::ceil(seconds))));
    updateParts();
}

void ProposalCard::setPreview(bool on) {
    const QSignalBlocker blocker(m_preview);
    m_preview->setChecked(on);
}

bool ProposalCard::previewOn() const { return m_preview->isChecked(); }

void ProposalCard::setPrimary(bool primary) { m_accept->setKind(primary && !m_compact ? ui::Button::Kind::Primary : ui::Button::Kind::Secondary); }

void ProposalCard::setAcceptTip(const QString &tip) {
    m_acceptTip = tip;
    if (!m_acceptBlocked) m_accept->setTip(m_acceptTip, m_acceptKey);
}

void ProposalCard::setAcceptKey(const QString &key) {
    m_acceptKey = key;
    if (!m_acceptBlocked) m_accept->setTip(m_acceptTip, m_acceptKey);
}

void ProposalCard::setGuest(bool guest) {
    m_guest = guest;
    updateParts();
}

void ProposalCard::setAcceptBlocked(const QString &reason) {
    m_accept->setEnabled(reason.isEmpty());
    m_acceptBlocked = !reason.isEmpty();
    if (m_acceptBlocked) m_accept->setTip(reason);
    else m_accept->setTip(m_acceptTip, m_acceptKey);
}

void ProposalCard::setActionsEnabled(bool enabled) {
    m_reject->setEnabled(enabled);
    m_preview->setEnabled(enabled);
    if (!enabled) m_accept->setEnabled(false);
}

void ProposalCard::setSettled(const QString &text, bool accepted) {
    m_settled = !text.isEmpty();
    m_settledLabel->setText(text);
    setInk(m_settledLabel, accepted ? accentInk : ink2);
    updateParts();
}

void ProposalCard::setCompact(bool compact) {
    m_compact = compact;
    m_watch->setKind(compact ? ui::Button::Kind::Primary : ui::Button::Kind::Secondary);
    m_accept->setKind(compact ? ui::Button::Kind::Secondary : ui::Button::Kind::Primary);
    updateParts();
}

void ProposalCard::updateParts() {
    const bool card = !m_settled;
    const bool marked = m_pinned && !m_compact;
    m_layout->setContentsMargins(card ? 12 : 0, card ? (marked ? 8 : 12) : 0, card ? 12 : 0, card ? 12 : 0);
    m_pinHeader->setVisible(card && marked);
    m_title->setVisible(card);
    m_second->setVisible(card);
    m_changes->setVisible(card && m_open);
    m_diff->parentWidget()->setVisible(card && m_open && m_script->isChecked());
    m_footer->setVisible(card && (m_open || m_compact));
    m_watchBox->setVisible(card && (m_open || m_compact) && m_hasWatch);
    m_settledLabel->setVisible(m_settled);
    m_reject->setVisible(!m_guest);
    m_accept->setVisible(!m_guest);
    m_wait->setVisible(m_guest);
    updateGeometry();
    update();
}

void ProposalCard::paintEvent(QPaintEvent *) {
    if (m_settled) return;
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF area(rect());
    p.setPen(Qt::NoPen);
    p.setBrush(color(raised));
    p.drawRoundedRect(area, radius::card, radius::card);
    if (m_pinned && !m_compact) {
        // inset 0 2px 0 accent: a 2 pt stripe inside the top edge, following its corners.
        p.save();
        QPainterPath clip;
        clip.addRoundedRect(area, radius::card, radius::card);
        p.setClipPath(clip);
        QPainterPath outer;
        outer.addRoundedRect(area, radius::card, radius::card);
        QPainterPath inner;
        inner.addRoundedRect(area.adjusted(0, 2, 0, 0), radius::card, radius::card);
        p.fillPath(outer.subtracted(inner), color(accent));
        p.restore();
    }
    ui::paintInsetLine(p, area, radius::card, color(line));
}

bool ProposalCard::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_title && event->type() == QEvent::MouseButtonRelease) {
        auto *mouse = static_cast<QMouseEvent *>(event);
        if (mouse->button() == Qt::LeftButton && m_title->rect().contains(mouse->position().toPoint())) {
            setOpen(!m_open);
            emit openToggled(m_open);
            return true;
        }
    }
    return QFrame::eventFilter(watched, event);
}

} // namespace kerf
