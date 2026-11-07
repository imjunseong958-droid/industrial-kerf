#include "effects_panel.h"
#include "inspector_presentation.h"
#include "design_tokens.h"
#include "editor_common.h"
#include "ui/components.h"
#include "ui/icons.h"
#include "ui/tray_shell.h"

#include <QAction>
#include <QElapsedTimer>
#include <QHBoxLayout>
#include <QImage>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QMimeData>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QScrollBar>
#include <QStyledItemDelegate>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>
#include <memory>
#include <utility>

namespace kerf {

using tokens::color;

namespace {

constexpr QSize kCard = EffectsPanel::kCard;
// Previews asked for at once; the rest wait until the cards narrow.
constexpr int kPreviewLimit = 48;
constexpr int kIdRole = Qt::UserRole;
constexpr int kCategoryRole = Qt::UserRole + 1;
constexpr int kSearchRole = Qt::UserRole + 2;
// Whether a card shows a picture rather than its placeholder.
constexpr int kHasPreview = Qt::UserRole + 3;
// "video", "audio" (an effect's domain) or "generator".
constexpr int kGroupRole = Qt::UserRole + 4;

// A card's thumb before (or without) a picture: black with the effect's
// symbol in ink3 (a sound effect's waveform, a generator's frame).
QPixmap placeholder(const QString &group, qreal ratio) {
    QPixmap pixmap(QSize(int(std::ceil(kCard.width() * ratio)), int(std::ceil(kCard.height() * ratio))));
    pixmap.setDevicePixelRatio(ratio);
    pixmap.fill(Qt::black);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    const QString icon = group == QStringLiteral("audio") ? QStringLiteral("g-audio")
                         : group == QStringLiteral("generator") ? QStringLiteral("g-video")
                                                               : QStringLiteral("g-fx");
    icons::paint(painter, icon, QRectF(QPointF(), QSizeF(kCard)), color(tokens::ink3), 16);
    return pixmap;
}

// A preview fitted at its own shape onto the black thumb.
QPixmap cardPixmap(const QImage &image, qreal ratio) {
    QPixmap pixmap(QSize(int(std::ceil(kCard.width() * ratio)), int(std::ceil(kCard.height() * ratio))));
    pixmap.setDevicePixelRatio(ratio);
    pixmap.fill(Qt::black);
    QPainter painter(&pixmap);
    const QSizeF fitted = QSizeF(image.size()).scaled(QSizeF(kCard), Qt::KeepAspectRatio);
    const QSize device(std::max(1, int(std::round(fitted.width() * ratio))), std::max(1, int(std::round(fitted.height() * ratio))));
    QImage scaled = image.scaled(device, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
    scaled.setDevicePixelRatio(ratio);
    painter.drawImage(QPointF(std::round((kCard.width() * ratio - device.width()) / 2.0) / ratio,
                              std::round((kCard.height() * ratio - device.height()) / 2.0) / ratio),
                      scaled);
    return pixmap;
}

// A card (mockup .ei): the thumb, radius 4, with a 1 pt line (lineh when
// hovered), and the name 4 under it in UI 11 ink2, cut with an ellipsis.
class CardDelegate final : public QStyledItemDelegate {
public:
    using QStyledItemDelegate::QStyledItemDelegate;
    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override {
        return QSize(kCard.width() + EffectsPanel::kColumnGap, kCard.height() + EffectsPanel::kNameTop + tokens::meta.line + EffectsPanel::kRowGap);
    }
    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        // QListView can ask for a row it has hidden, at the first card's place
        // (its name then prints over that card's).
        if (const auto *view = qobject_cast<const QListView *>(option.widget); view && view->isRowHidden(index.row())) return;
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing);
        const QRectF thumb(option.rect.left(), option.rect.top(), kCard.width(), kCard.height());
        QPainterPath round;
        round.addRoundedRect(thumb, tokens::radius::small, tokens::radius::small);
        painter->save();
        painter->setClipPath(round);
        const QIcon icon = qvariant_cast<QIcon>(index.data(Qt::DecorationRole));
        painter->drawPixmap(thumb.toRect(), icon.pixmap(kCard, painter->device()->devicePixelRatioF()));
        painter->restore();
        const bool hovered = option.state & QStyle::State_MouseOver;
        const bool selected = option.state & QStyle::State_Selected;
        ui::paintInsetLine(*painter, thumb, tokens::radius::small,
                           color(selected ? tokens::ink : hovered ? tokens::lineh : tokens::line), selected ? 1.5 : 1);
        painter->setFont(tokens::font(tokens::meta));
        painter->setPen(color(selected ? tokens::ink : tokens::ink2));
        const QRectF name(thumb.left(), thumb.bottom() + EffectsPanel::kNameTop, thumb.width(), tokens::meta.line);
        painter->drawText(name, Qt::AlignLeft | Qt::AlignVCenter,
                          QFontMetrics(painter->font()).elidedText(index.data(Qt::DisplayRole).toString(), Qt::ElideRight, int(name.width())));
        painter->restore();
    }
};

// The cards drag as their effect's id, which the Inspector (and a clip)
// drops into the clip's stack where it lands; title cards drag as their
// generator's id, which a video track takes. The pointer over a thumb
// scrubs it; a 32 pt fade covers the bottom while there is more below.
class EffectCards final : public QListWidget {
public:
    using QListWidget::QListWidget;
    std::function<void(const QString &fx, double fraction)> scrub;
    std::function<void()> unscrub;
    QString mime = QString::fromLatin1(EffectsPanel::kFxMime);

protected:
    QStringList mimeTypes() const override { return {mime}; }
    QMimeData *mimeData(const QList<QListWidgetItem *> &items) const override {
        if (items.isEmpty()) return nullptr;
        auto *data = new QMimeData;
        data->setData(mime, items.first()->data(kIdRole).toString().toUtf8());
        data->setText(items.first()->text());
        return data;
    }
    void mouseMoveEvent(QMouseEvent *event) override {
        QListWidget::mouseMoveEvent(event);
        if (event->buttons() != Qt::NoButton) return;
        const QPoint at = event->position().toPoint();
        QListWidgetItem *item = itemAt(at);
        const QRect cell = item ? visualItemRect(item) : QRect();
        const QRect thumb(cell.topLeft(), kCard);
        if (item && thumb.contains(at)) {
            if (scrub) scrub(item->data(kIdRole).toString(), std::clamp(double(at.x() - thumb.left()) / thumb.width(), 0.0, 1.0));
        } else if (unscrub) {
            unscrub();
        }
    }
    bool viewportEvent(QEvent *event) override {
        if (event->type() == QEvent::Leave && unscrub) unscrub();
        return QListWidget::viewportEvent(event);
    }
    void paintEvent(QPaintEvent *event) override {
        QListWidget::paintEvent(event);
        if (verticalScrollBar()->value() >= verticalScrollBar()->maximum()) return;
        QPainter painter(viewport());
        QLinearGradient fade(0, viewport()->height() - ui::FadeScrollArea::bottomFade, 0, viewport()->height());
        QColor clear = color(tokens::chrome);
        clear.setAlpha(0);
        fade.setColorAt(0, clear);
        fade.setColorAt(1, color(tokens::chrome));
        painter.fillRect(QRect(0, viewport()->height() - ui::FadeScrollArea::bottomFade, viewport()->width(), ui::FadeScrollArea::bottomFade), fade);
    }
};

// A generator's name as the catalog writes it is its doc's first sentence, which says what it does
// and goes in its tip. Its card says what it is.
QString generatorName(const QString &id) {
    return id.isEmpty() ? id : id.at(0).toUpper() + id.mid(1);
}

} // namespace

EffectsPanel::EffectsPanel(QWidget *parent, Mode mode) : QWidget(parent), m_mode(mode) {
    setObjectName(mode == Mode::Effects ? QStringLiteral("effectsPanel") : QStringLiteral("titlesPanel"));
    auto elapsed = std::make_shared<QElapsedTimer>();
    elapsed->start();
    m_clock = [elapsed] { return elapsed->elapsed(); };
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    // The search field, shown by Search (⌘⇧F).
    m_searchRow = new QWidget;
    m_searchRow->setObjectName(QStringLiteral("browserSearchRow"));
    m_searchRow->setFixedHeight(tokens::layout::strip);
    auto *searchLayout = new QHBoxLayout(m_searchRow);
    searchLayout->setContentsMargins(kPadX, 2, kPadX, 2);
    m_search = new QLineEdit;
    m_search->setObjectName(mode == Mode::Effects ? QStringLiteral("effectsSearch") : QStringLiteral("titlesSearch"));
    m_search->setPlaceholderText(mode == Mode::Effects ? QStringLiteral("Search effects") : QStringLiteral("Search titles"));
    m_search->setAccessibleName(m_search->placeholderText());
    m_search->setClearButtonEnabled(true);
    m_search->setFont(tokens::font(tokens::control));
    m_search->installEventFilter(this);
    searchLayout->addWidget(m_search);
    m_searchRow->hide();
    layout->addWidget(m_searchRow);

    // The category row: 24 tall, 4 under the strip, pad 0 12 0 4.
    auto *row = new QWidget;
    row->setObjectName(QStringLiteral("browserCategoryRow"));
    row->setFixedHeight(kCategoryRow + tokens::space::s4);
    auto *rowLayout = new QHBoxLayout(row);
    rowLayout->setContentsMargins(tokens::space::s4, tokens::space::s4, kPadX, 0);
    rowLayout->setSpacing(0);
    m_categoryButton = new ui::GhostButton;
    m_categoryButton->setObjectName(QStringLiteral("browserCategory"));
    m_categoryButton->setChevron(true);
    m_categoryButton->setTip(mode == Mode::Effects ? QStringLiteral("Category") : QStringLiteral("Title category"));
    rowLayout->addWidget(m_categoryButton);
    rowLayout->addStretch(1);
    m_count = shell::text(QString(), tokens::meta, tokens::ink3);
    m_count->setObjectName(QStringLiteral("browserCount"));
    rowLayout->addWidget(m_count);
    layout->addWidget(row);
    connect(m_categoryButton, &QAbstractButton::clicked, this, [this] {
        QMenu *menu = categoryMenu();
        ui::popupMenu(menu, m_categoryButton);
    });
    // The status line: what the engine answered an add with.
    m_status = new QLabel;
    m_status->setFont(tokens::font(tokens::meta));
    m_status->setObjectName(QStringLiteral("browserStatus"));
    m_status->setWordWrap(true);
    m_status->setContentsMargins(kPadX, tokens::space::s4, kPadX, 0);
    m_status->hide();
    layout->addWidget(m_status);

    m_hint = shell::text(QString(), tokens::meta, tokens::ink3);
    m_hint->setObjectName(QStringLiteral("browserEmpty"));
    m_hint->setAlignment(Qt::AlignCenter);
    m_hint->setWordWrap(true);
    m_hint->setContentsMargins(kPadX, kPadY, kPadX, kPadY);
    layout->addWidget(m_hint, 1);

    auto *cards = new EffectCards;
    m_cards = cards;
    m_cards->setObjectName(mode == Mode::Effects ? QStringLiteral("effectsCards") : QStringLiteral("titlesCards"));
    m_cards->setItemDelegate(new CardDelegate(m_cards));
    if (mode == Mode::Titles) cards->mime = QString::fromLatin1(kGeneratorMime);
    m_cards->setDragEnabled(true);
    m_cards->setDragDropMode(QAbstractItemView::DragOnly);
    m_cards->setViewMode(QListView::IconMode);
    m_cards->setMovement(QListView::Static);
    m_cards->setResizeMode(QListView::Adjust);
    m_cards->setFlow(QListView::LeftToRight);
    m_cards->setWrapping(true);
    m_cards->setSpacing(0);
    m_cards->setUniformItemSizes(true);
    m_cards->setIconSize(kCard);
    m_cards->setGridSize(QSize(kCard.width() + kColumnGap, kCard.height() + kNameTop + tokens::meta.line + kRowGap));
    m_cards->setFrameShape(QFrame::NoFrame);
    m_cards->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_cards->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_cards->setMouseTracking(true);
    m_cards->setStyleSheet(QStringLiteral("QListWidget { background: transparent; border: none; outline: none; }"));
    m_cards->viewport()->setAutoFillBackground(false);
    m_cards->setContentsMargins(0, 0, 0, 0);
    if (mode == Mode::Effects) {
        cards->scrub = [this](const QString &fx, double fraction) {
            if (!m_gate || m_gate()) scrubAt(fx, fraction);
        };
        cards->unscrub = [this] { endScrub(); };
    }
    auto *grid = new QWidget;
    auto *gridLayout = new QVBoxLayout(grid);
    // Pad 8 12: the cells carry the column gap on their right, so the
    // first column sits 12 in and the third ends 12 + 3 x 96 - 8 in.
    gridLayout->setContentsMargins(kPadX, kPadY, 0, 0);
    gridLayout->addWidget(m_cards);
    layout->addWidget(grid, 1);

    m_previewTimer = new QTimer(this);
    m_previewTimer->setSingleShot(true);
    m_previewTimer->setInterval(150);
    // Only a page on screen asks on its own; one behind another page asks
    // when it is shown.
    connect(m_previewTimer, &QTimer::timeout, this, [this] {
        if (isVisible() && (!m_gate || m_gate())) requestPreviews();
    });
    m_scrubTimer = new QTimer(this);
    m_scrubTimer->setSingleShot(true);
    connect(m_scrubTimer, &QTimer::timeout, this, &EffectsPanel::sendScrub);
    connect(m_search, &QLineEdit::textChanged, this, &EffectsPanel::refilter);
    connect(m_cards, &QListWidget::itemActivated, this, &EffectsPanel::add);
    setCatalog({});
    setClip(QString(), false);
}

void EffectsPanel::schedulePreviews() {
    m_previewTimer->start();
}

void EffectsPanel::setPreviewGate(std::function<bool()> gate) {
    m_gate = std::move(gate);
}

void EffectsPanel::setClock(std::function<qint64()> clock) {
    if (clock) m_clock = std::move(clock);
}

qint64 EffectsPanel::now() const {
    return m_clock ? m_clock() : 0;
}

void EffectsPanel::setCatalog(const QJsonArray &catalog) {
    m_cards->clear();
    m_items.clear();
    m_fx.clear();
    m_base.clear();
    const qreal ratio = std::max<qreal>(1.0, devicePixelRatioF());
    for (const QJsonValue &value : catalog) {
        const QJsonObject fx = value.toObject();
        const QString kind = fx.value(QStringLiteral("kind")).toString();
        const QString domain = fx.value(QStringLiteral("domain")).toString(QStringLiteral("video"));
        if (m_mode == Mode::Effects ? kind != QStringLiteral("effect") : kind != QStringLiteral("generator")) continue;
        const QString id = fx.value(QStringLiteral("id")).toString();
        const QString category = fx.value(QStringLiteral("category")).toString();
        const QString group = m_mode == Mode::Titles ? QStringLiteral("generator") : domain;
        // As the Inspector's rows name it, once it is on a clip.
        const QString name = m_mode == Mode::Titles ? generatorName(id) : presentationEffectName(fx.value(QStringLiteral("name")).toString());
        auto *item = new QListWidgetItem(placeholder(group, ratio), name);
        item->setData(kIdRole, id);
        item->setData(kCategoryRole, category);
        item->setData(kGroupRole, group);
        item->setData(kHasPreview, false);
        item->setData(kSearchRole, QStringLiteral("%1 %2 %3 %4").arg(id, category, name, fx.value(QStringLiteral("doc")).toString()).toLower());
        m_cards->addItem(item);
        m_items.insert(id, item);
        m_fx.insert(id, fx);
    }
    rebuildCategories();
    updateTips();
    refilter();
}

void EffectsPanel::rebuildCategories() {
    const QString previous = category();
    m_categories.clear();
    QHash<QString, int> counts;
    QStringList order;
    int video = 0;
    int audio = 0;
    for (int row = 0; row < m_cards->count(); ++row) {
        const QListWidgetItem *item = m_cards->item(row);
        const QString group = item->data(kGroupRole).toString();
        if (group == QStringLiteral("audio")) {
            ++audio;
            continue;
        }
        ++video;
        const QString category = item->data(kCategoryRole).toString();
        if (!counts.contains(category)) order << category;
        ++counts[category];
    }
    if (m_mode == Mode::Effects) {
        // All video effects, the picture categories in catalog order, then
        // all sound effects (the catalog's audio domain).
        m_categories.append({QStringLiteral("video"), QStringLiteral("All video effects"), video, false});
        for (const QString &name : std::as_const(order))
            m_categories.append({QStringLiteral("video:") + name, name.isEmpty() ? QStringLiteral("Other") : name, counts.value(name), false});
        m_categories.append({QStringLiteral("audio"), QStringLiteral("All audio effects"), audio, true});
    } else {
        m_categories.append({QStringLiteral("generator"), QStringLiteral("All titles"), video, false});
        if (order.size() > 1)
            for (const QString &name : std::as_const(order))
                m_categories.append({QStringLiteral("generator:") + name, name.isEmpty() ? QStringLiteral("Other") : name, counts.value(name), false});
    }
    const auto kept = std::find_if(m_categories.cbegin(), m_categories.cend(), [&previous](const Category &c) { return c.key == previous; });
    m_categoryKey = kept != m_categories.cend() ? previous : m_categories.isEmpty() ? QString() : m_categories.constFirst().key;
}

QString EffectsPanel::category() const {
    return m_categoryKey;
}

void EffectsPanel::setCategory(const QString &key) {
    if (key == m_categoryKey) return;
    if (std::none_of(m_categories.cbegin(), m_categories.cend(), [&key](const Category &c) { return c.key == key; })) return;
    m_categoryKey = key;
    refilter();
}

QMenu *EffectsPanel::categoryMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("browserCategoryMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    ui::styleMenu(menu);
    const QString current = category();
    for (const Category &entry : std::as_const(m_categories)) {
        if (entry.separatorBefore) menu->addSeparator();
        QAction *action = menu->addAction(entry.name);
        action->setCheckable(true);
        action->setChecked(entry.key == current);
        action->setData(entry.key);
        ui::setMenuKey(action, QString::number(entry.count));
        const QString key = entry.key;
        connect(action, &QAction::triggered, this, [this, key] { setCategory(key); });
    }
    return menu;
}

void EffectsPanel::setClip(const QString &clip, bool picture) {
    if (clip == m_clip && picture == m_picture) return;
    const QString before = previewClip();
    m_clip = clip;
    m_picture = picture;
    if (previewClip() != before) resetPreviews();
    m_previewTimer->start();
}

void EffectsPanel::setProgramClip(const QString &clip) {
    if (clip == m_programClip) return;
    const QString before = previewClip();
    m_programClip = clip;
    if (previewClip() != before) {
        resetPreviews();
        m_previewTimer->start();
    }
}

QString EffectsPanel::previewClip() const {
    // Generators are drawn alone, on the program sequence's canvas.
    if (m_mode == Mode::Titles) return QString();
    return m_picture && !m_clip.isEmpty() ? m_clip : m_programClip;
}

void EffectsPanel::resetPreviews() {
    // Generators are drawn alone on black, whatever the clip.
    if (m_mode == Mode::Titles) return;
    m_base.clear();
    const qreal ratio = std::max<qreal>(1.0, devicePixelRatioF());
    for (QListWidgetItem *item : std::as_const(m_items)) {
        item->setIcon(placeholder(item->data(kGroupRole).toString(), ratio));
        item->setData(kHasPreview, false);
    }
}

void EffectsPanel::setCardImage(QListWidgetItem *item, const QImage &image) {
    item->setIcon(cardPixmap(image, std::max<qreal>(1.0, devicePixelRatioF())));
}

bool EffectsPanel::ownsPreviewKey(const QString &key) const {
    if (m_mode == Mode::Titles) return key == QLatin1String(kTitleKey);
    return key == QLatin1String(kCardKey) || key == QLatin1String(kScrubKey);
}

void EffectsPanel::setPreview(const QString &clip, const QString &fx, const QString &key, const QImage &image) {
    // Only the keys this page asks with: any other answer (a proposal's
    // "before:"/"after:" frame, the other page's) has someone else's params.
    if (image.isNull() || !ownsPreviewKey(key)) return;
    QListWidgetItem *item = m_items.value(fx);
    if (!item) return;
    const bool scrub = key == QLatin1String(kScrubKey);
    if (m_mode == Mode::Effects && clip != previewClip()) return;
    if (scrub) {
        m_scrubDue.remove(fx);
        if (fx == m_scrubFx) setCardImage(item, image);
        return;
    }
    m_base.insert(fx, image);
    item->setData(kHasPreview, true);
    if (fx != m_scrubFx) setCardImage(item, image);
}

void EffectsPanel::setAddAvailable(bool available, const QString &reason) {
    m_addAvailable = available;
    m_addReason = available ? QString() : reason;
    updateTips();
}

void EffectsPanel::setStatus(const QString &message, bool error) {
    m_statusError = error && !message.isEmpty();
    m_status->setText(message);
    // Its own sheet: the app's `*` rule would give it the text colour.
    m_status->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: %2px; background: transparent; }")
                                .arg(color(m_statusError ? tokens::neg : tokens::ink3).name(QColor::HexRgb))
                                .arg(tokens::meta.px));
    m_status->setVisible(!message.isEmpty());
}

QString EffectsPanel::status() const {
    return m_status->text();
}

QString EffectsPanel::displayName(const QString &fx) const {
    const QListWidgetItem *item = m_items.value(fx);
    return item ? item->text() : fx;
}

void EffectsPanel::updateTips() {
    for (QListWidgetItem *item : std::as_const(m_items)) {
        const QJsonObject fx = m_fx.value(item->data(kIdRole).toString());
        QStringList tip{item->text()};
        tip.first() += m_mode == Mode::Effects ? QStringLiteral(" · drag to a clip") : QStringLiteral(" · double-click or drag to a video track");
        const QString doc = fx.value(QStringLiteral("doc")).toString();
        if (!doc.isEmpty() && doc != item->text()) tip << doc;
        if (!m_addAvailable && !m_addReason.isEmpty()) tip << m_addReason;
        item->setToolTip(tip.join(QLatin1Char('\n')));
    }
}

void EffectsPanel::refilter() {
    const QString key = category();
    const QString group = key.section(QLatin1Char(':'), 0, 0);
    const bool narrowed = key.contains(QLatin1Char(':'));
    const QString wanted = key.section(QLatin1Char(':'), 1);
    const QStringList words = m_search->text().toLower().split(QLatin1Char(' '), Qt::SkipEmptyParts);
    for (QListWidgetItem *item : std::as_const(m_items)) {
        const QString text = item->data(kSearchRole).toString();
        const bool inGroup = group.isEmpty() || item->data(kGroupRole).toString() == group;
        const bool inCategory = !narrowed || item->data(kCategoryRole).toString() == wanted;
        const bool shown = inGroup && inCategory &&
                           std::all_of(words.begin(), words.end(), [&text](const QString &word) { return text.contains(word); });
        item->setHidden(!shown);
    }
    const int shown = shownCount();
    m_count->setText(shown == 1 ? QStringLiteral("1 item") : QStringLiteral("%1 items").arg(shown));
    const auto at = std::find_if(m_categories.cbegin(), m_categories.cend(), [&key](const Category &c) { return c.key == key; });
    m_categoryButton->setText(at != m_categories.cend() ? at->name : m_mode == Mode::Effects ? QStringLiteral("All video effects")
                                                                                             : QStringLiteral("All titles"));
    // A ghost button is as wide as it was last told to be.
    m_categoryButton->setFixedSize(m_categoryButton->sizeHint());
    m_categoryButton->setEnabled(!m_items.isEmpty());
    // The empty states: no catalog (no engine), or nothing matches.
    if (m_items.isEmpty()) {
        m_hint->setText(m_mode == Mode::Effects ? QStringLiteral("No effects: the Kerf engine's catalog is not available")
                                                : QStringLiteral("No titles: the Kerf engine's catalog is not available"));
    } else if (shown == 0) {
        m_hint->setText(words.isEmpty() ? QStringLiteral("Nothing in this category")
                                        : QStringLiteral("Nothing matches “%1”").arg(m_search->text().trimmed()));
    }
    m_hint->setVisible(m_items.isEmpty() || shown == 0);
    m_cards->setVisible(!m_hint->isVisible());
    m_previewTimer->start();
}

int EffectsPanel::shownCount() const {
    int shown = 0;
    for (const QListWidgetItem *item : std::as_const(m_items)) shown += item->isHidden() ? 0 : 1;
    return shown;
}

QStringList EffectsPanel::shownEffects() const {
    QStringList shown;
    for (int row = 0; row < m_cards->count() && shown.size() < kPreviewLimit; ++row) {
        const QListWidgetItem *item = m_cards->item(row);
        // Sound effects and the adjustment layer have no picture.
        if (item->isHidden() || item->data(kHasPreview).toBool() || item->data(kGroupRole).toString() == QStringLiteral("audio") ||
            item->data(kIdRole).toString() == QStringLiteral("adjustment"))
            continue;
        shown << item->data(kIdRole).toString();
    }
    return shown;
}

void EffectsPanel::requestPreviews() {
    const QStringList effects = shownEffects();
    if (effects.isEmpty()) return;
    if (m_mode == Mode::Titles) {
        emit previewsWanted(previewClip(), effects);
        return;
    }
    if (!previewClip().isEmpty()) emit previewsWanted(previewClip(), effects);
}

void EffectsPanel::add(QListWidgetItem *item) {
    if (!item || !m_addAvailable) return;
    if (m_mode == Mode::Effects && m_clip.isEmpty()) return;
    emit addRequested(item->data(kIdRole).toString());
}

void EffectsPanel::focusSearch() {
    m_searchRow->show();
    m_search->setFocus(Qt::OtherFocusReason);
    m_search->selectAll();
}

QJsonObject EffectsPanel::mainParam(const QJsonObject &fx) {
    const QJsonArray params = fx.value(QStringLiteral("params")).toArray();
    const auto ranged = [](const QJsonObject &param) {
        return param.value(QStringLiteral("type")).toString() == QStringLiteral("number") && param.value(QStringLiteral("min")).isDouble() &&
               param.value(QStringLiteral("max")).isDouble() &&
               param.value(QStringLiteral("max")).toDouble() > param.value(QStringLiteral("min")).toDouble();
    };
    const auto shape = [](const QJsonObject &param) {
        return QJsonObject{{QStringLiteral("name"), param.value(QStringLiteral("name"))},
                           {QStringLiteral("min"), param.value(QStringLiteral("min"))},
                           {QStringLiteral("max"), param.value(QStringLiteral("max"))}};
    };
    for (const char *preferred : {"amount", "mix", "intensity", "strength", "opacity", "radius", "size"})
        for (const QJsonValue &value : params) {
            const QJsonObject param = value.toObject();
            if (param.value(QStringLiteral("name")).toString() == QLatin1String(preferred) && ranged(param)) return shape(param);
        }
    for (const QJsonValue &value : params)
        if (ranged(value.toObject())) return shape(value.toObject());
    return {};
}

void EffectsPanel::scrubAt(const QString &fx, double fraction) {
    if (m_mode != Mode::Effects || previewClip().isEmpty() || mainParam(m_fx.value(fx)).isEmpty()) {
        endScrub();
        return;
    }
    if (fx != m_scrubFx) endScrub();
    m_scrubFx = fx;
    m_scrubFraction = std::clamp(fraction, 0.0, 1.0);
    // At most one request per 120 ms: now, or when the interval is up, with
    // the latest position.
    const qint64 since = now() - m_lastScrub;
    if (since >= kScrubIntervalMs) {
        m_scrubTimer->stop();
        sendScrub();
    } else if (!m_scrubTimer->isActive()) {
        m_scrubTimer->start(int(kScrubIntervalMs - since));
    }
}

void EffectsPanel::sendScrub() {
    if (m_scrubFx.isEmpty()) return;
    const QJsonObject param = mainParam(m_fx.value(m_scrubFx));
    if (param.isEmpty()) return;
    const double min = param.value(QStringLiteral("min")).toDouble();
    const double max = param.value(QStringLiteral("max")).toDouble();
    const double value = min + (max - min) * m_scrubFraction;
    m_lastScrub = now();
    m_scrubDue.insert(m_scrubFx);
    emit scrubWanted(previewClip(), m_scrubFx, QJsonObject{{param.value(QStringLiteral("name")).toString(), value}});
}

void EffectsPanel::endScrub() {
    m_scrubTimer->stop();
    if (m_scrubFx.isEmpty()) return;
    const QString fx = std::exchange(m_scrubFx, QString());
    if (QListWidgetItem *item = m_items.value(fx)) {
        if (m_base.contains(fx)) setCardImage(item, m_base.value(fx));
        else item->setIcon(placeholder(item->data(kGroupRole).toString(), std::max<qreal>(1.0, devicePixelRatioF())));
    }
    // A newer request stops an older one: the thumbs the scrub cut short
    // are asked for again.
    m_previewTimer->start();
}

void EffectsPanel::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), color(tokens::chrome));
    if (m_searchRow->isVisible()) painter.fillRect(QRect(0, m_searchRow->geometry().bottom(), width(), 1), color(tokens::line));
}

void EffectsPanel::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    m_previewTimer->start();
}

bool EffectsPanel::eventFilter(QObject *watched, QEvent *event) {
    // Esc in the search field clears it and puts it away.
    if (watched == m_search && event->type() == QEvent::KeyPress && static_cast<QKeyEvent *>(event)->key() == Qt::Key_Escape) {
        m_search->clear();
        m_searchRow->hide();
        m_cards->setFocus(Qt::OtherFocusReason);
        update();
        return true;
    }
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
