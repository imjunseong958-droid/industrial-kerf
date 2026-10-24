#include "effect_controls_panel.h"
#include "editor_common.h"

#include <QAction>
#include <QCheckBox>
#include <QColorDialog>
#include <QComboBox>
#include <QDoubleSpinBox>
#include <QFileDialog>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QPixmap>
#include <QPushButton>
#include <QScrollArea>
#include <QSignalBlocker>
#include <QStyle>
#include <QToolButton>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

EffectControlsPanel::EffectControlsPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("effectControls"));
    setAttribute(Qt::WA_StyledBackground);
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(0, 0, 0, 0);
    auto *scroll = new QScrollArea;
    scroll->setFrameShape(QFrame::NoFrame);
    scroll->setWidgetResizable(true);
    scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    outer->addWidget(scroll);
    auto *content = new QWidget;
    scroll->setWidget(content);
    auto *layout = new QVBoxLayout(content);
    layout->setContentsMargins(10, 8, 10, 8);
    layout->setSpacing(6);
    auto *timing = new QWidget;
    auto *form = new QFormLayout(timing);
    form->setContentsMargins(0, 0, 0, 0);
    form->setSpacing(6);
    m_clipName = new QLabel(QStringLiteral("Select a clip"));
    m_clipName->setObjectName(QStringLiteral("inspectorClip"));
    form->addRow(m_clipName);
    m_position = timingSpin(0.0, 86400.0, 0.01, QStringLiteral("timelineStart"));
    m_duration = timingSpin(0.01, 86400.0, 0.01, QStringLiteral("duration"));
    m_in = timingSpin(0.0, 86400.0, 0.01, QStringLiteral("in"));
    m_out = timingSpin(0.01, 86400.0, 0.01, QStringLiteral("out"));
    m_speed = timingSpin(0.01, 20.0, 0.01, QStringLiteral("speed"));
    form->addRow(QStringLiteral("Position"), m_position);
    form->addRow(QStringLiteral("Duration"), m_duration);
    form->addRow(QStringLiteral("Source in"), m_in);
    form->addRow(QStringLiteral("Source out"), m_out);
    form->addRow(QStringLiteral("Speed"), m_speed);
    layout->addWidget(timing);
    auto *properties = new QWidget;
    m_propertyForm = new QFormLayout(properties);
    m_propertyForm->setContentsMargins(0, 0, 0, 0);
    m_propertyForm->setSpacing(6);
    layout->addWidget(properties);
    auto *effects = new QWidget;
    effects->setObjectName(QStringLiteral("inspectorEffects"));
    m_effectsLayout = new QVBoxLayout(effects);
    m_effectsLayout->setContentsMargins(0, 6, 0, 0);
    m_effectsLayout->setSpacing(8);
    layout->addWidget(effects);
    layout->addStretch(1);
    setClip(QString(), QJsonObject{}, QJsonArray{}, m_fps);
}

QDoubleSpinBox *EffectControlsPanel::timingSpin(double minimum, double maximum, double step, const QString &field) {
    auto *spin = new QDoubleSpinBox;
    spin->setRange(minimum, maximum);
    spin->setSingleStep(step);
    spin->setDecimals(2);
    spin->setProperty("clipField", field);
    connect(spin, &QDoubleSpinBox::editingFinished, this, &EffectControlsPanel::timingEdited);
    return spin;
}

void EffectControlsPanel::setClip(const QString &id, const QJsonObject &clip, const QJsonArray &defs, double fps) {
    m_updating = true;
    m_clipId = id;
    m_clip = clip;
    m_fps = fps;
    const bool hasClip = !id.isEmpty();
    m_clipName->setText(hasClip ? clipName(clip) : QStringLiteral("Select a clip"));
    auto setSpin = [hasClip](QDoubleSpinBox *spin, double value) {
        spin->setEnabled(hasClip);
        const QSignalBlocker blocker(spin);
        spin->setValue(value);
    };
    setSpin(m_position, clip.value(QStringLiteral("timelineStart")).toDouble());
    setSpin(m_duration, std::max(0.01, clip.value(QStringLiteral("duration")).toDouble(1.0)));
    setSpin(m_in, clip.value(QStringLiteral("in")).toDouble());
    setSpin(m_out, std::max(0.01, clip.value(QStringLiteral("out")).toDouble(1.0)));
    setSpin(m_speed, clip.value(QStringLiteral("speed")).toDouble(1.0));
    // Property editors follow the catalog definitions; animated properties
    // show their first key and are read-only.
    if (defs != m_propertyDefs) {
        while (m_propertyForm->rowCount() > 0) m_propertyForm->removeRow(0);
        m_propertyEditors.clear();
        m_propertyDefs = defs;
        for (const QJsonValue &def : defs) {
            QWidget *editor = propertyEditor(def.toObject());
            const QString name = def.toObject().value(QStringLiteral("name")).toString();
            m_propertyEditors.insert(name, editor);
            m_propertyForm->addRow(name, editor);
        }
    }
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    for (const QJsonValue &def : defs) {
        const QString name = def.toObject().value(QStringLiteral("name")).toString();
        const QJsonValue prop = props.value(name);
        QWidget *editor = m_propertyEditors.value(name);
        setPropertyValue(editor, prop.isArray() ? prop.toArray().first().toObject().value(QStringLiteral("value"))
                                 : prop.isUndefined() ? def.toObject().value(QStringLiteral("default")) : prop);
        editor->setEnabled(!prop.isArray());
    }
    showEffects();
    m_updating = false;
}

void EffectControlsPanel::setEffectCatalog(const QJsonArray &catalog) {
    m_effectDefs.clear();
    for (const QJsonValue &value : catalog) {
        const QJsonObject fx = value.toObject();
        m_effectDefs.insert(fx.value(QStringLiteral("id")).toString(), fx);
    }
    m_effectsShown.clear();
}

void EffectControlsPanel::setPictureTracks(const QStringList &tracks) {
    if (tracks == m_pictureTracks) return;
    m_pictureTracks = tracks;
    m_effectsShown.clear();
}

void EffectControlsPanel::showEffects() {
    const QJsonArray effects = m_clip.value(QStringLiteral("effects")).toArray();
    QStringList stack{m_clipId};
    for (const QJsonValue &effect : effects) stack << effect.toObject().value(QStringLiteral("fx")).toString();
    const QString shown = stack.join(QLatin1Char('|'));
    if (shown == m_effectsShown) {
        // The same stack: refresh the values in place, keeping focus.
        for (int index = 0; index < effects.size(); ++index) {
            const QJsonObject effect = effects.at(index).toObject();
            const QJsonObject params = effect.value(QStringLiteral("params")).toObject();
            QWidget *box = m_effectsLayout->itemAt(index) ? m_effectsLayout->itemAt(index)->widget() : nullptr;
            if (!box) continue;
            if (auto *enabled = box->findChild<QCheckBox *>(QStringLiteral("effectEnabled"))) {
                const QSignalBlocker blocker(enabled);
                enabled->setChecked(effect.value(QStringLiteral("enabled")).toBool(true));
            }
            for (const QJsonValue &def : m_effectDefs.value(effect.value(QStringLiteral("fx")).toString())
                                            .value(QStringLiteral("params"))
                                            .toArray()) {
                const QString name = def.toObject().value(QStringLiteral("name")).toString();
                QWidget *editor = box->findChild<QWidget *>(QStringLiteral("effectParam_") + name);
                if (!editor) continue;
                const QJsonValue value = params.value(name);
                setPropertyValue(editor, value.isArray() ? value.toArray().first().toObject().value(QStringLiteral("value"))
                                         : value.isUndefined() ? def.toObject().value(QStringLiteral("default")) : value);
                editor->setEnabled(!value.isArray());
            }
        }
        return;
    }
    m_effectsShown = shown;
    while (QLayoutItem *item = m_effectsLayout->takeAt(0)) {
        delete item->widget();
        delete item;
    }
    for (int index = 0; index < effects.size(); ++index) {
        const QJsonObject effect = effects.at(index).toObject();
        const QString fx = effect.value(QStringLiteral("fx")).toString();
        const QJsonObject def = m_effectDefs.value(fx);
        auto *box = new QWidget;
        box->setObjectName(QStringLiteral("effectBox"));
        auto *boxLayout = new QVBoxLayout(box);
        boxLayout->setContentsMargins(0, 0, 0, 0);
        boxLayout->setSpacing(4);
        auto *header = new QHBoxLayout;
        auto *enabled = new QCheckBox(def.value(QStringLiteral("name")).toString(fx));
        enabled->setObjectName(QStringLiteral("effectEnabled"));
        enabled->setChecked(effect.value(QStringLiteral("enabled")).toBool(true));
        enabled->setToolTip(def.value(QStringLiteral("doc")).toString(effect.value(QStringLiteral("doc")).toString()));
        connect(enabled, &QCheckBox::clicked, this, [this, index](bool on) {
            if (m_clipId.isEmpty() || m_updating) return;
            edit(QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), m_clipId},
                             {QStringLiteral("index"), index}, {QStringLiteral("enabled"), on}});
        });
        auto *remove = new QToolButton;
        remove->setObjectName(QStringLiteral("effectRemove"));
        remove->setText(QStringLiteral("✕"));
        remove->setToolTip(QStringLiteral("Remove this effect"));
        connect(remove, &QToolButton::clicked, this, [this, index] {
            if (m_clipId.isEmpty()) return;
            edit(QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-remove")}, {QStringLiteral("clip"), m_clipId},
                             {QStringLiteral("index"), index}});
        });
        header->addWidget(enabled, 1);
        header->addWidget(remove);
        boxLayout->addLayout(header);
        auto *form = new QFormLayout;
        form->setSpacing(4);
        const QJsonObject params = effect.value(QStringLiteral("params")).toObject();
        for (const QJsonValue &paramValue : def.value(QStringLiteral("params")).toArray()) {
            const QJsonObject param = paramValue.toObject();
            const QString name = param.value(QStringLiteral("name")).toString();
            const QJsonValue fallback = param.value(QStringLiteral("default"));
            QWidget *editor = valueEditor(param, name, [this, index, name, fallback](const QJsonValue &value) {
                if (m_clipId.isEmpty() || m_updating) return;
                const bool isDefault = value.isDouble() && fallback.isDouble()
                                           ? std::abs(value.toDouble() - fallback.toDouble()) < 5e-4
                                           : value == fallback;
                edit(QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), m_clipId},
                                 {QStringLiteral("index"), index},
                                 {QStringLiteral("params"), QJsonObject{{name, isDefault ? QJsonValue() : value}}}});
            });
            editor->setObjectName(QStringLiteral("effectParam_") + name);
            const QJsonValue value = params.value(name);
            setPropertyValue(editor, value.isArray() ? value.toArray().first().toObject().value(QStringLiteral("value"))
                                     : value.isUndefined() ? fallback : value);
            editor->setEnabled(!value.isArray());
            QString label = param.value(QStringLiteral("doc")).toString(name);
            if (label.isEmpty()) label = name;
            form->addRow(label, editor);
        }
        boxLayout->addLayout(form);
        m_effectsLayout->addWidget(box);
    }
}

void EffectControlsPanel::edit(const QJsonObject &op) {
    if (!op.isEmpty()) emit editRequested(QJsonArray{op});
}

void EffectControlsPanel::timingEdited() {
    if (m_clipId.isEmpty() || m_updating) return;
    const QString field = sender()->property("clipField").toString();
    const auto *spin = qobject_cast<QDoubleSpinBox *>(sender());
    if (!spin || field.isEmpty()) return;
    const double value = spin->value();
    const double speed = std::max(0.01, m_clip.value(QStringLiteral("speed")).toDouble(1.0));
    const auto frames = [this](const QString &key) { return static_cast<qint64>(m_clip.value(key).toDouble()); };
    const auto frameAt = [this](double seconds) { return std::llround(seconds * m_fps); };
    auto amount = [this](const QString &op, double count, const QString &edge = QString()) {
        const qint64 whole = std::llround(count);
        if (whole == 0) return QJsonObject{};
        QJsonObject edit{{QStringLiteral("op"), op}, {QStringLiteral("clip"), m_clipId},
                         {QStringLiteral("frames"), whole}};
        if (!edge.isEmpty()) edit.insert(QStringLiteral("edge"), edge);
        return edit;
    };
    if (field == QStringLiteral("timelineStart")) {
        edit(amount(QStringLiteral("move"), double(frameAt(value) - frames(QStringLiteral("startFrame")))));
    } else if (field == QStringLiteral("duration")) {
        edit(amount(QStringLiteral("trim"), double(frameAt(value) - frames(QStringLiteral("durationFrames"))), QStringLiteral("out")));
    } else if (field == QStringLiteral("in")) {
        edit(amount(QStringLiteral("slip"), (value - m_clip.value(QStringLiteral("in")).toDouble()) / speed * m_fps));
    } else if (field == QStringLiteral("out")) {
        edit(amount(QStringLiteral("trim"), (value - m_clip.value(QStringLiteral("out")).toDouble()) / speed * m_fps, QStringLiteral("out")));
    } else if (field == QStringLiteral("speed")) {
        QString ratio = QString::number(value, 'f', 4);
        while (ratio.endsWith(QLatin1Char('0'))) ratio.chop(1);
        if (ratio.endsWith(QLatin1Char('.'))) ratio.chop(1);
        if (m_clip.value(QStringLiteral("reverse")).toBool()) ratio.prepend(QLatin1Char('-'));
        edit(QJsonObject{{QStringLiteral("op"), QStringLiteral("speed")}, {QStringLiteral("clip"), m_clipId},
                         {QStringLiteral("speed"), ratio}});
    }
}

void EffectControlsPanel::propertyEdited(const QString &name, const QJsonValue &value) {
    if (m_clipId.isEmpty() || m_updating) return;
    QJsonValue fallback;
    for (const QJsonValue &def : m_propertyDefs)
        if (def.toObject().value(QStringLiteral("name")).toString() == name) fallback = def.toObject().value(QStringLiteral("default"));
    const QJsonValue current = m_clip.value(QStringLiteral("props")).toObject().value(name);
    auto same = [](const QJsonValue &a, const QJsonValue &b) {
        return a.isDouble() && b.isDouble() ? std::abs(a.toDouble() - b.toDouble()) < 5e-4 : a == b;
    };
    if (same(value, current.isUndefined() ? fallback : current)) return;
    edit(QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), m_clipId},
                     {QStringLiteral("props"), QJsonObject{{name, same(value, fallback) ? QJsonValue() : value}}}});
}

QWidget *EffectControlsPanel::propertyEditor(const QJsonObject &def) {
    const QString name = def.value(QStringLiteral("name")).toString();
    return valueEditor(def, name, [this, name](const QJsonValue &value) { propertyEdited(name, value); });
}

QWidget *EffectControlsPanel::valueEditor(const QJsonObject &def, const QString &title,
                                          std::function<void(const QJsonValue &)> edited) {
    const QString type = def.value(QStringLiteral("type")).toString();
    QWidget *editor = nullptr;
    if (type == QStringLiteral("number")) {
        auto *spin = new QDoubleSpinBox;
        const double minimum = def.value(QStringLiteral("min")).toDouble(-1e6);
        const double maximum = def.value(QStringLiteral("max")).toDouble(1e6);
        spin->setRange(minimum, maximum);
        spin->setDecimals(3);
        spin->setSingleStep(maximum - minimum > 10.0 ? 1.0 : 0.01);
        connect(spin, &QDoubleSpinBox::editingFinished, this, [spin, edited] { edited(spin->value()); });
        editor = spin;
    } else if (type == QStringLiteral("enum")) {
        auto *combo = new QComboBox;
        for (const QJsonValue &option : def.value(QStringLiteral("options")).toArray()) combo->addItem(option.toString());
        connect(combo, &QComboBox::activated, this, [combo, edited] { edited(combo->currentText()); });
        editor = combo;
    } else if (type == QStringLiteral("bool")) {
        auto *check = new QCheckBox;
        connect(check, &QCheckBox::clicked, this, [edited](bool checked) { edited(checked); });
        editor = check;
    } else if (type == QStringLiteral("color")) {
        auto *button = new QPushButton;
        connect(button, &QPushButton::clicked, this, [this, button, title, edited] {
            const QColor color = QColorDialog::getColor(modelColor(button->text()), this, title,
                                                        QColorDialog::ShowAlphaChannel);
            if (color.isValid()) edited(modelColorName(color));
        });
        editor = button;
    } else if (type == QStringLiteral("track")) {
        // Another picture track, or none (the empty first entry).
        auto *combo = new QComboBox;
        combo->addItem(QString());
        combo->addItems(m_pictureTracks);
        connect(combo, &QComboBox::activated, this, [combo, edited] { edited(combo->currentText()); });
        editor = combo;
    } else if (type == QStringLiteral("file")) {
        // A path, typed or chosen; relative paths are relative to the project.
        auto *line = new QLineEdit;
        QStringList patterns;
        for (const QJsonValue &extension : def.value(QStringLiteral("extensions")).toArray())
            patterns << QStringLiteral("*.%1").arg(extension.toString());
        const QString filter = QStringLiteral("%1 (%2)").arg(title, patterns.join(QLatin1Char(' ')));
        QAction *choose = line->addAction(style()->standardIcon(QStyle::SP_DirOpenIcon), QLineEdit::TrailingPosition);
        choose->setToolTip(tr("Choose a file"));
        connect(choose, &QAction::triggered, this, [this, line, title, filter, edited] {
            const QString path = QFileDialog::getOpenFileName(this, title, line->text(), filter);
            if (path.isEmpty()) return;
            line->setText(path);
            edited(path);
        });
        connect(line, &QLineEdit::editingFinished, this, [line, edited] { edited(line->text()); });
        editor = line;
    } else {
        auto *line = new QLineEdit;
        connect(line, &QLineEdit::editingFinished, this, [line, edited] { edited(line->text()); });
        editor = line;
    }
    QString tip = def.value(QStringLiteral("doc")).toString();
    if (def.value(QStringLiteral("unit")).isString()) tip += QStringLiteral(" (%1)").arg(def.value(QStringLiteral("unit")).toString());
    editor->setToolTip(tip.trimmed());
    return editor;
}

void EffectControlsPanel::setPropertyValue(QWidget *editor, const QJsonValue &value) {
    const QSignalBlocker blocker(editor);
    if (auto *spin = qobject_cast<QDoubleSpinBox *>(editor)) spin->setValue(value.toDouble());
    else if (auto *combo = qobject_cast<QComboBox *>(editor)) combo->setCurrentText(value.toString());
    else if (auto *check = qobject_cast<QCheckBox *>(editor)) check->setChecked(value.toBool());
    else if (auto *button = qobject_cast<QPushButton *>(editor)) {
        QPixmap swatch(12, 12);
        swatch.fill(modelColor(value.toString()));
        button->setIcon(swatch);
        button->setText(value.toString());
    }
    else if (auto *line = qobject_cast<QLineEdit *>(editor)) line->setText(value.toString());
}

} // namespace kerf
